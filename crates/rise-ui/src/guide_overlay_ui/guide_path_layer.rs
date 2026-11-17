use gpui::{Bounds, Hsla, PathBuilder, Pixels, Point, Window, point, px, size, transparent_black};
use rise_core::foundation::foundation_date::RiseClock;

use crate::guide_overlay_ui::guide_overlay_theme::GuideOverlayTheme;
use crate::guide_overlay_ui::guide_spotlight_layer::append_rounded_rect;
use crate::swiftui::animated::Animated;
use crate::swiftui::motion::{MotionSpec, duration_from_seconds};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum GuidePathElement {
    MoveTo(Point<Pixels>),
    LineTo(Point<Pixels>),
    QuadCurveTo { to: Point<Pixels>, control: Point<Pixels> },
}

/// the path a route builds, one open polyline with rounded corners
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GuidePathRoute {
    pub elements: Vec<GuidePathElement>,
}

impl GuidePathRoute {
    const CURVE_STEPS: usize = 8;

    /// route as a polyline with corners sampled into chords
    /// stroke reveal walks this by length
    pub fn flattened(&self) -> Vec<Point<Pixels>> {
        let mut points: Vec<Point<Pixels>> = Vec::new();
        for element in &self.elements {
            match *element {
                GuidePathElement::MoveTo(to) | GuidePathElement::LineTo(to) => points.push(to),
                GuidePathElement::QuadCurveTo { to, control } => {
                    let Some(&from) = points.last() else {
                        points.push(to);
                        continue;
                    };
                    for step in 1..=Self::CURVE_STEPS {
                        let t = step as f32 / Self::CURVE_STEPS as f32;
                        let inverse = 1.0 - t;
                        let x = inverse * inverse * from.x.as_f32()
                            + 2.0 * inverse * t * control.x.as_f32()
                            + t * t * to.x.as_f32();
                        let y = inverse * inverse * from.y.as_f32()
                            + 2.0 * inverse * t * control.y.as_f32()
                            + t * t * to.y.as_f32();
                        points.push(point(px(x), px(y)));
                    }
                }
            }
        }
        points
    }
}

pub struct GuidePathLayer {
    points: Vec<Point<Pixels>>,
    stroke_color: Hsla,
    line_width: f32,
    stroke_end: Animated<f32>,
    opacity: f32,
    presented_opacity: Animated<f32>,
}

impl Default for GuidePathLayer {
    fn default() -> Self {
        Self::new()
    }
}

impl GuidePathLayer {
    pub fn new() -> Self {
        Self {
            points: Vec::new(),
            stroke_color: transparent_black(),
            line_width: GuideOverlayTheme::PATH_LINE_WIDTH,
            stroke_end: Animated::new(0.0),
            opacity: 0.0,
            presented_opacity: Animated::new(0.0),
        }
    }

    fn motion(duration: f64) -> MotionSpec {
        MotionSpec::curve(GuideOverlayTheme::core_animation_timing(), duration_from_seconds(duration))
    }

    /// draws the line between two cutout rects
    /// animated false still shows the finished line, reduce motion removes travel not info
    pub fn draw(&mut self, source: Bounds<Pixels>, target: Bounds<Pixels>, color: Hsla, animated: bool) {
        let Some(route) = Self::route(
            source,
            target,
            GuideOverlayTheme::PATH_CORNER_RADIUS,
            GuideOverlayTheme::PATH_ANCHOR_GAP,
        ) else {
            self.clear(false);
            return;
        };

        self.stroke_color = color;
        self.line_width = GuideOverlayTheme::PATH_LINE_WIDTH;
        self.points = route.flattened();
        self.opacity = 1.0;
        self.presented_opacity.jump(1.0);

        if !animated {
            self.stroke_end.jump(1.0);
            return;
        }
        self.stroke_end.jump(0.0);
        let spec = Self::motion(GuideOverlayTheme::PATH_DRAW);
        self.stroke_end.set(1.0, spec, RiseClock::monotonic(), false);
    }

    /// removes the line, animated it fades with the scrim
    pub fn clear(&mut self, animated: bool) {
        let finished = *self.stroke_end.target();
        self.stroke_end.jump(finished);
        if !animated || self.opacity <= 0.0 {
            self.points.clear();
            self.stroke_end.jump(0.0);
            self.opacity = 0.0;
            self.presented_opacity.jump(0.0);
            return;
        }

        self.opacity = 0.0;
        let spec = Self::motion(GuideOverlayTheme::SCRIM_FADE);
        self.presented_opacity.set(0.0, spec, RiseClock::monotonic(), false);
    }

    /// paints the visible part of the line, asks for a frame only while it moves
    pub fn paint(&self, window: &mut Window) {
        if self.points.len() < 2 {
            return;
        }
        let now = RiseClock::monotonic();
        let opacity = self.presented_opacity.frame_value(now, window);
        let stroke_end = self.stroke_end.frame_value(now, window).clamp(0.0, 1.0);
        if opacity <= 0.0 || stroke_end <= 0.0 {
            return;
        }

        let visible = Self::prefix(&self.points, stroke_end);
        if visible.len() < 2 {
            return;
        }
        let mut color = self.stroke_color;
        color.a *= opacity;

        let mut builder = PathBuilder::stroke(px(self.line_width));
        builder.move_to(visible[0]);
        for &next in &visible[1..] {
            builder.line_to(next);
        }
        if let Ok(path) = builder.build() {
            window.paint_path(path, color);
        }

        let side = self.line_width;
        for end in [visible[0], visible[visible.len() - 1]] {
            let mut cap = PathBuilder::fill();
            let dot = Bounds::new(
                point(px(end.x.as_f32() - side / 2.0), px(end.y.as_f32() - side / 2.0)),
                size(px(side), px(side)),
            );
            append_rounded_rect(&mut cap, dot, side / 2.0);
            if let Ok(path) = cap.build() {
                window.paint_path(path, color);
            }
        }
    }

    /// first fraction of the polyline by length
    fn prefix(points: &[Point<Pixels>], fraction: f32) -> Vec<Point<Pixels>> {
        let segment_length = |a: Point<Pixels>, b: Point<Pixels>| {
            (b.x.as_f32() - a.x.as_f32()).hypot(b.y.as_f32() - a.y.as_f32())
        };
        let total: f32 = points.windows(2).map(|pair| segment_length(pair[0], pair[1])).sum();
        if fraction >= 1.0 || total <= 0.0 {
            return points.to_vec();
        }
        let mut remaining = total * fraction;
        let mut visible = vec![points[0]];
        for pair in points.windows(2) {
            let length = segment_length(pair[0], pair[1]);
            if length <= 0.0 {
                continue;
            }
            if remaining >= length {
                visible.push(pair[1]);
                remaining -= length;
                continue;
            }
            let ratio = remaining / length;
            let x = pair[0].x.as_f32() + (pair[1].x.as_f32() - pair[0].x.as_f32()) * ratio;
            let y = pair[0].y.as_f32() + (pair[1].y.as_f32() - pair[0].y.as_f32()) * ratio;
            visible.push(point(px(x), px(y)));
            break;
        }
        visible
    }

    /// right angled route between two rects with rounded corners
    /// dominant axis picks the exit edge and the turn is at the halfway point
    pub fn route(
        source: Bounds<Pixels>,
        target: Bounds<Pixels>,
        corner_radius: f32,
        gap: f32,
    ) -> Option<GuidePathRoute> {
        let (s_x, s_y) = (source.origin.x.as_f32(), source.origin.y.as_f32());
        let (s_w, s_h) = (source.size.width.as_f32(), source.size.height.as_f32());
        let (t_x, t_y) = (target.origin.x.as_f32(), target.origin.y.as_f32());
        let (t_w, t_h) = (target.size.width.as_f32(), target.size.height.as_f32());
        if !(s_w > 0.0 && s_h > 0.0 && t_w > 0.0 && t_h > 0.0) || source.intersects(&target) {
            return None;
        }
        let (s_mid_x, s_mid_y) = (s_x + s_w / 2.0, s_y + s_h / 2.0);
        let (t_mid_x, t_mid_y) = (t_x + t_w / 2.0, t_y + t_h / 2.0);
        let at = |x: f32, y: f32| point(px(x), px(y));

        let dx = t_mid_x - s_mid_x;
        let dy = t_mid_y - s_mid_y;
        let points: Vec<Point<Pixels>> = if dy.abs() >= dx.abs() {
            let downwards = dy >= 0.0;
            let start_y = if downwards { s_y + s_h + gap } else { s_y - gap };
            let end_y = if downwards { t_y - gap } else { t_y + t_h + gap };
            let sign = if downwards { 1.0 } else { -1.0 };
            if (end_y - start_y) * sign <= 0.0 {
                return None;
            }
            if (t_mid_x - s_mid_x).abs() < 1.0 {
                vec![at(s_mid_x, start_y), at(t_mid_x, end_y)]
            } else {
                let turn = (start_y + end_y) / 2.0;
                vec![
                    at(s_mid_x, start_y),
                    at(s_mid_x, turn),
                    at(t_mid_x, turn),
                    at(t_mid_x, end_y),
                ]
            }
        } else {
            let rightwards = dx >= 0.0;
            let start_x = if rightwards { s_x + s_w + gap } else { s_x - gap };
            let end_x = if rightwards { t_x - gap } else { t_x + t_w + gap };
            let sign = if rightwards { 1.0 } else { -1.0 };
            if (end_x - start_x) * sign <= 0.0 {
                return None;
            }
            if (t_mid_y - s_mid_y).abs() < 1.0 {
                vec![at(start_x, s_mid_y), at(end_x, t_mid_y)]
            } else {
                let turn = (start_x + end_x) / 2.0;
                vec![
                    at(start_x, s_mid_y),
                    at(turn, s_mid_y),
                    at(turn, t_mid_y),
                    at(end_x, t_mid_y),
                ]
            }
        };

        Self::rounded_polyline(&points, corner_radius)
    }

    fn rounded_polyline(points: &[Point<Pixels>], corner_radius: f32) -> Option<GuidePathRoute> {
        if points.len() < 2 {
            return None;
        }
        let mut elements = vec![GuidePathElement::MoveTo(points[0])];

        for index in 1..(points.len() - 1) {
            let previous = points[index - 1];
            let corner = points[index];
            let next = points[index + 1];
            let radius = corner_radius.min(Self::distance(previous, corner).min(Self::distance(corner, next)) / 2.0);
            let entry = Self::point(corner, previous, radius);
            let exit = Self::point(corner, next, radius);
            match (radius > 0.0, entry, exit) {
                (true, Some(entry), Some(exit)) => {
                    elements.push(GuidePathElement::LineTo(entry));
                    elements.push(GuidePathElement::QuadCurveTo { to: exit, control: corner });
                }
                _ => elements.push(GuidePathElement::LineTo(corner)),
            }
        }

        elements.push(GuidePathElement::LineTo(points[points.len() - 1]));
        Some(GuidePathRoute { elements })
    }

    fn distance(lhs: Point<Pixels>, rhs: Point<Pixels>) -> f32 {
        (rhs.x.as_f32() - lhs.x.as_f32()).hypot(rhs.y.as_f32() - lhs.y.as_f32())
    }

    fn point(origin: Point<Pixels>, other: Point<Pixels>, length: f32) -> Option<Point<Pixels>> {
        let span = Self::distance(origin, other);
        if span <= 0.0 {
            return None;
        }
        let ratio = length / span;
        Some(point(
            px(origin.x.as_f32() + (other.x.as_f32() - origin.x.as_f32()) * ratio),
            px(origin.y.as_f32() + (other.y.as_f32() - origin.y.as_f32()) * ratio),
        ))
    }
}

/// host view for the path layer, non interactive
#[derive(Default)]
pub struct GuidePathView {
    pub path_layer: GuidePathLayer,
}

impl GuidePathView {
    pub fn new() -> Self {
        Self::default()
    }
}
