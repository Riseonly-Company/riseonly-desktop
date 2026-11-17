use std::f32::consts::PI;

use gpui::{
    BorderStyle, Bounds, Corners, Edges, Hsla, IntoElement, PathBuilder, Pixels, Point, Styled, Window, canvas, point,
    px, quad, size,
};
use rise_canon::RiseGuideGesture;
use rise_core::foundation::foundation_date::RiseClock;

use crate::guide_overlay_ui::guide_attention_layer::{GuideRingPath, paint_stroked_ring};
use crate::guide_overlay_ui::guide_overlay_theme::GuideOverlayTheme;
use crate::guide_overlay_ui::guide_spotlight_layer::GuideSpotlightCutout;
use crate::swiftui::animated::{Animated, MotionTime};
use crate::swiftui::motion::{CubicBezier, MotionSpec, duration_from_seconds};

/// what the figure was last built for, rebuilt only when one of these changes
#[derive(Clone, Copy, Debug, PartialEq)]
struct GuideGestureDrawing {
    gesture: RiseGuideGesture,
    travel: f32,
    animated: bool,
}

/// animated hint a step draws in its hole, which way to swipe or where to hold or double tap
/// a plain bubble saying hold reads as tap
/// reduce motion keeps the picture and drops the travel
#[derive(Clone, Debug)]
pub struct GuideGestureLayer {
    gesture: Option<RiseGuideGesture>,
    drawn_for: Option<GuideGestureDrawing>,
    figure: Option<GuideGestureDrawing>,
    centre: Point<Pixels>,
    opacity: Animated<f32>,
    parts_start: Option<MotionTime>,
}

impl Default for GuideGestureLayer {
    fn default() -> Self {
        Self::new()
    }
}

impl GuideGestureLayer {
    pub fn new() -> Self {
        Self {
            gesture: None,
            drawn_for: None,
            figure: None,
            centre: Point::default(),
            opacity: Animated::new(0.0),
            parts_start: None,
        }
    }

    pub fn gesture(&self) -> Option<RiseGuideGesture> {
        self.gesture
    }

    /// puts the hint in the hole, moves it with the hole or removes it
    /// rebuilds the figure only when gesture, travel or motion setting changed
    pub fn show(
        &mut self,
        gesture: Option<RiseGuideGesture>,
        cutout: Option<GuideSpotlightCutout>,
        animated: bool,
        fades: bool,
    ) {
        let now = RiseClock::monotonic();
        let (Some(gesture), Some(cutout)) = (gesture, cutout) else {
            self.gesture = None;
            self.set_visible(false, fades, now);
            return;
        };
        self.gesture = Some(gesture);
        let rect = cutout.resolved_rect();
        self.centre = rect.center();

        let travel = Self::travel(gesture, rect);
        let drawing = GuideGestureDrawing {
            gesture,
            travel,
            animated,
        };
        if self.drawn_for != Some(drawing) {
            self.parts_start = Some(now);
            self.figure = Some(drawing);
            self.drawn_for = Some(drawing);
        }
        self.set_visible(true, fades, now);
    }

    /// stops every loop, called when the overlay goes away so nothing animates unseen
    pub fn stop(&mut self) {
        self.gesture = None;
        self.drawn_for = None;
        self.figure = None;
        self.parts_start = None;
        self.opacity.jump(0.0);
    }

    /// true when the hint needs another frame at now
    pub fn is_animating(&self, now: MotionTime) -> bool {
        if self.opacity.is_running(now) {
            return true;
        }
        *self.opacity.target() > 0.0
            && self.parts_start.is_some()
            && self.figure.is_some_and(|figure| figure.animated)
    }

    /// chevrons point up in figure space, the figure is turned for the other three
    fn angle(gesture: RiseGuideGesture) -> f32 {
        match gesture {
            RiseGuideGesture::SwipeUp => 0.0,
            RiseGuideGesture::SwipeRight => PI / 2.0,
            RiseGuideGesture::SwipeDown => PI,
            RiseGuideGesture::SwipeLeft => -PI / 2.0,
            RiseGuideGesture::Hold | RiseGuideGesture::DoubleTap => 0.0,
        }
    }

    /// long hole gets long travel and a button sized one short travel, so chevrons stay in the light
    fn travel(gesture: RiseGuideGesture, rect: Bounds<Pixels>) -> f32 {
        let length = match gesture {
            RiseGuideGesture::SwipeUp | RiseGuideGesture::SwipeDown => f32::from(rect.size.height),
            RiseGuideGesture::SwipeLeft | RiseGuideGesture::SwipeRight => f32::from(rect.size.width),
            RiseGuideGesture::Hold | RiseGuideGesture::DoubleTap => return 0.0,
        };
        let range = GuideOverlayTheme::GESTURE_TRAVEL_RANGE;
        let wanted = length * GuideOverlayTheme::GESTURE_TRAVEL_SHARE;
        wanted.max(*range.start()).min(*range.end()).round()
    }

    fn set_visible(&mut self, visible: bool, animated: bool, now: MotionTime) {
        let target = if visible { 1.0 } else { 0.0 };
        if *self.opacity.target() == target {
            return;
        }
        if !visible {
            self.drawn_for = None;
            if !animated {
                self.figure = None;
                self.parts_start = None;
            }
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

    /// draws the figure in host coordinates, asks for the next frame while it moves
    pub fn paint(&self, bounds: Bounds<Pixels>, window: &mut Window) {
        let now = RiseClock::monotonic();
        if self.is_animating(now) {
            window.request_animation_frame();
        }
        let opacity = self.opacity.value(now);
        let (Some(start), Some(gesture)) = (self.parts_start, self.figure) else {
            return;
        };
        if opacity <= 0.0 {
            return;
        }
        let elapsed = now.saturating_sub(start).as_secs_f64();
        let figure = GuideGestureFigure {
            centre: point(self.centre.x + bounds.origin.x, self.centre.y + bounds.origin.y),
            angle: Self::angle(gesture.gesture),
            opacity,
        };
        match gesture.gesture {
            RiseGuideGesture::SwipeUp
            | RiseGuideGesture::SwipeDown
            | RiseGuideGesture::SwipeLeft
            | RiseGuideGesture::SwipeRight => figure.paint_swipe(gesture.travel, gesture.animated, elapsed, window),
            RiseGuideGesture::Hold => figure.paint_hold(gesture.animated, elapsed, window),
            RiseGuideGesture::DoubleTap => figure.paint_double_tap(gesture.animated, elapsed, window),
        }
    }
}

struct GuideGestureFigure {
    centre: Point<Pixels>,
    angle: f32,
    opacity: f32,
}

impl GuideGestureFigure {
    /// turns a point of figure space into a screen point
    fn place(&self, x: f32, y: f32) -> Point<Pixels> {
        let (sin, cos) = self.angle.sin_cos();
        point(
            self.centre.x + px(x * cos - y * sin),
            self.centre.y + px(x * sin + y * cos),
        )
    }

    fn paint_swipe(&self, travel: f32, animated: bool, elapsed: f64, window: &mut Window) {
        let width = GuideOverlayTheme::GESTURE_CHEVRON_WIDTH;
        let depth = GuideOverlayTheme::GESTURE_CHEVRON_DEPTH;
        let count = GuideOverlayTheme::GESTURE_CHEVRON_COUNT;
        let spacing = GuideOverlayTheme::GESTURE_CHEVRON_SPACING;
        let cycle = GuideOverlayTheme::GESTURE_TRAVEL
            + GuideOverlayTheme::GESTURE_STAGGER * (count - 1) as f64
            + GuideOverlayTheme::GESTURE_REST;
        let local = loop_time(elapsed, cycle);

        for index in 0..count {
            let offset = ((count - 1) as f32 / 2.0 - index as f32) * spacing;
            let (translation, part_opacity) = if animated {
                let stagger = GuideOverlayTheme::GESTURE_STAGGER * (count - 1 - index) as f64;
                let duration = GuideOverlayTheme::GESTURE_TRAVEL;
                let child = local - stagger;
                if !(0.0..duration).contains(&child) {
                    continue;
                }
                let fraction = child / duration;
                let eased = CubicBezier::EASE_IN_OUT.progress(fraction) as f32;
                let translation = travel / 2.0 + (-travel) * eased;
                let fade = keyframe(&[0.0, 1.0, 1.0, 0.0], &[0.0, 0.25, 0.6, 1.0], fraction);
                (translation, fade)
            } else {
                (0.0, 0.45 + 0.55 * index as f32 / (count - 1).max(1) as f32)
            };
            let y = offset + translation;
            let points = [
                self.place(-width / 2.0, y + depth / 2.0),
                self.place(0.0, y - depth / 2.0),
                self.place(width / 2.0, y + depth / 2.0),
            ];
            self.stroke_part(&points, false, part_opacity, window);
        }
    }

    fn paint_hold(&self, animated: bool, elapsed: f64, window: &mut Window) {
        let finger_side = GuideOverlayTheme::GESTURE_FINGER_SIDE;
        let ring_side = GuideOverlayTheme::GESTURE_HOLD_RING_SIDE;

        if !animated {
            self.circle_part(finger_side, 1.0, 1.0, true, window);
            self.arc_part(ring_side / 2.0, 1.0, 1.0, window);
            return;
        }

        let press = GuideOverlayTheme::GESTURE_HOLD_FILL * 0.2;
        let fill = GuideOverlayTheme::GESTURE_HOLD_FILL;
        let release = GuideOverlayTheme::GESTURE_REST;
        let cycle = press + fill + release + GuideOverlayTheme::GESTURE_REST;
        let local = loop_time(elapsed, cycle);
        let fraction = local / cycle;

        let sink = keyframe(
            &[1.12, 0.9, 0.9, 1.0],
            &[0.0, press / cycle, (press + fill) / cycle, 1.0],
            fraction,
        );
        let fade = keyframe(
            &[0.0, 1.0, 1.0, 0.0, 0.0],
            &[
                0.0,
                press / cycle,
                (press + fill) / cycle,
                (press + fill + release) / cycle,
                1.0,
            ],
            fraction,
        );
        self.circle_part(finger_side, sink, fade, true, window);

        let grow = ((local - press) / fill).clamp(0.0, 1.0);
        let stroke_end = CubicBezier::EASE_IN_OUT.progress(grow) as f32;
        self.arc_part(ring_side / 2.0, stroke_end, fade, window);
    }

    fn paint_double_tap(&self, animated: bool, elapsed: f64, window: &mut Window) {
        let finger_side = GuideOverlayTheme::GESTURE_FINGER_SIDE;
        let side = GuideOverlayTheme::GESTURE_TAP_RIPPLE_SIDE;

        if !animated {
            self.circle_part(finger_side, 1.0, 1.0, true, window);
            self.circle_part(side, 0.8, 1.0, false, window);
            self.circle_part(side, 1.0, 0.5, false, window);
            return;
        }

        let tap = GuideOverlayTheme::GESTURE_TAP_RIPPLE;
        let gap = GuideOverlayTheme::GESTURE_STAGGER * 1.4;
        let cycle = tap + gap + GuideOverlayTheme::GESTURE_REST * 2.4;
        let local = loop_time(elapsed, cycle);

        let dip = keyframe(
            &[1.0, 0.86, 1.0, 0.86, 1.0, 1.0],
            &[
                0.0,
                (tap * 0.25) / cycle,
                gap / cycle,
                (gap + tap * 0.25) / cycle,
                (gap * 2.0) / cycle,
                1.0,
            ],
            local / cycle,
        );
        self.circle_part(finger_side, dip, 1.0, true, window);

        for index in 0..2_u32 {
            let start = f64::from(index) * gap;
            let child = local - start;
            if child < 0.0 {
                continue;
            }
            if child >= tap {
                continue;
            }
            let fraction = child / tap;
            let scale = 0.55 + (1.35 - 0.55) * CubicBezier::EASE_OUT.progress(fraction) as f32;
            let opacity = 0.95 + (0.0 - 0.95) * fraction as f32;
            self.circle_part(side, scale, opacity, false, window);
        }
    }

    /// stroked open polyline with round caps and joins and a soft dark edge
    fn stroke_part(&self, points: &[Point<Pixels>], closed: bool, part_opacity: f32, window: &mut Window) {
        let opacity = self.opacity * part_opacity;
        if opacity <= 0.0 || points.len() < 2 {
            return;
        }
        let line_width = GuideOverlayTheme::GESTURE_LINE_WIDTH;
        let shadow_width = line_width + GuideOverlayTheme::GESTURE_SHADOW_RADIUS;
        let shadow = GuideOverlayTheme::gesture_shadow_color().opacity(opacity);
        let color = GuideOverlayTheme::gesture_color().opacity(opacity);
        stroke_polyline(points, shadow_width, closed, shadow, window);
        stroke_polyline(points, line_width, closed, color, window);
    }

    /// circle of side around the center scaled by scale, the finger is filled
    fn circle_part(&self, side: f32, scale: f32, part_opacity: f32, filled: bool, window: &mut Window) {
        let opacity = self.opacity * part_opacity;
        if opacity <= 0.0 {
            return;
        }
        let diameter = side * scale;
        let rect = Bounds::new(
            point(self.centre.x - px(diameter / 2.0), self.centre.y - px(diameter / 2.0)),
            size(px(diameter), px(diameter)),
        );
        let color = GuideOverlayTheme::gesture_color();
        if filled {
            window.paint_quad(quad(
                rect,
                Corners::all(px(diameter / 2.0)),
                color.opacity(GuideOverlayTheme::GESTURE_FINGER_FILL_ALPHA * opacity),
                Edges::all(px(0.0)),
                color.opacity(0.0),
                BorderStyle::Solid,
            ));
        }
        let ring = GuideRingPath {
            rect,
            corner_radius: diameter / 2.0,
        };
        let line_width = GuideOverlayTheme::GESTURE_LINE_WIDTH * scale;
        paint_stroked_ring(
            ring,
            line_width + GuideOverlayTheme::GESTURE_SHADOW_RADIUS * scale,
            GuideOverlayTheme::gesture_shadow_color().opacity(opacity),
            Point::default(),
            window,
        );
        paint_stroked_ring(ring, line_width, color.opacity(opacity), Point::default(), window);
    }

    /// hold ring cut to stroke_end, from twelve oclock clockwise
    fn arc_part(&self, radius: f32, stroke_end: f32, part_opacity: f32, window: &mut Window) {
        if stroke_end <= 0.0 {
            return;
        }
        let sweep = 2.0 * PI * stroke_end.min(1.0);
        let steps = ((sweep / (2.0 * PI)) * 96.0).ceil().max(2.0) as usize;
        let start = -PI / 2.0;
        let points: Vec<Point<Pixels>> = (0..=steps)
            .map(|step| {
                let angle = start + sweep * step as f32 / steps as f32;
                point(
                    self.centre.x + px(radius * angle.cos()),
                    self.centre.y + px(radius * angle.sin()),
                )
            })
            .collect();
        self.stroke_part(&points, false, part_opacity, window);
    }
}

/// time inside one repetition of a loop that started elapsed seconds ago
fn loop_time(elapsed: f64, cycle: f64) -> f64 {
    if cycle <= 0.0 { 0.0 } else { elapsed % cycle }
}

/// linear keyframe track sampled at fraction of its duration
fn keyframe(values: &[f32], key_times: &[f64], fraction: f64) -> f32 {
    let Some((&first, rest)) = values.split_first() else {
        return 0.0;
    };
    if fraction <= key_times.first().copied().unwrap_or(0.0) {
        return first;
    }
    let mut previous_value = first;
    let mut previous_time = key_times.first().copied().unwrap_or(0.0);
    for (value, time) in rest.iter().zip(key_times.iter().skip(1)) {
        if fraction <= *time {
            let span = *time - previous_time;
            if span <= 0.0 {
                return *value;
            }
            let local = ((fraction - previous_time) / span) as f32;
            return previous_value + (*value - previous_value) * local;
        }
        previous_value = *value;
        previous_time = *time;
    }
    previous_value
}

/// stroke along points with round ends and joins
/// gpui strokes have butt ends and mitre joins so each vertex gets a dot
fn stroke_polyline(points: &[Point<Pixels>], line_width: f32, closed: bool, color: Hsla, window: &mut Window) {
    let mut builder = PathBuilder::stroke(px(line_width));
    builder.add_polygon(points, closed);
    if let Ok(path) = builder.build() {
        window.paint_path(path, color);
    }
    let half = line_width / 2.0;
    let dots: Vec<&Point<Pixels>> = if points.len() > 3 {
        points.first().into_iter().chain(points.last()).collect()
    } else {
        points.iter().collect()
    };
    for at in dots {
        let bounds = Bounds::new(point(at.x - px(half), at.y - px(half)), size(px(line_width), px(line_width)));
        window.paint_quad(quad(
            bounds,
            Corners::all(px(half)),
            color,
            Edges::all(px(0.0)),
            color.opacity(0.0),
            BorderStyle::Solid,
        ));
    }
}

/// host view for the hint
/// never interactive and registers no hitbox because the hole belongs to the app
#[derive(Clone, Debug, Default)]
pub struct GuideGestureView {
    pub gesture_layer: GuideGestureLayer,
}

impl GuideGestureView {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn gesture_layer(&self) -> &GuideGestureLayer {
        &self.gesture_layer
    }

    pub fn gesture_layer_mut(&mut self) -> &mut GuideGestureLayer {
        &mut self.gesture_layer
    }

    /// element that draws the hint over the whole overlay for this frame
    pub fn element(&self) -> impl IntoElement {
        let layer = self.gesture_layer.clone();
        canvas(
            |_, _, _| (),
            move |bounds, (), window, _| layer.paint(bounds, window),
        )
        .absolute()
        .size_full()
    }
}
