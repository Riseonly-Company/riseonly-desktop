use std::f64::consts::PI;

use gpui::{
    AnyElement, App, Bounds, FontWeight, Hsla, IntoElement, ParentElement, PathBuilder, Pixels, Point, Rgba, Styled,
    Window, div, fill, linear_color_stop, linear_gradient, point, px, size,
};
use rise_core::rise_app_events::{RiseNotificationName, UIAccessibility, UIApplication};
use rise_platform::power::{POWER_STATE_DID_CHANGE, system_low_power_mode};
use rise_ui::icon_ui::IconUi;
use rise_ui::swiftui::animated::{Animated, MotionTime};
use rise_ui::swiftui::environment::reduce_motion;
use rise_ui::swiftui::font::{Font, SwiftUiFont};
use rise_ui::swiftui::motion::{CubicBezier, MotionSpec, ease_in_out, ease_out, linear, spring, timing_curve};

use crate::core::lib::canon_format::RiseFormat;
use crate::modules::map::components::pins::map_friend_pin_view::MapPinPalette;
use crate::modules::map::shared::map_friend_status::{MapPinTile, MapPinTrail};

/// implicit transaction animation with the default curve and no timing function
fn implicit_transaction(duration: f64) -> MotionSpec {
    timing_curve(0.25, 0.1, 0.25, 1.0, duration)
}

/// autoreversing endless ease in ease out animation with duration per leg
/// sampled elapsed seconds after start, 0 at the start value and 1 at the target
pub(crate) fn autoreversing_progress(elapsed: f64, duration: f64) -> f64 {
    if duration <= 0.0 {
        return 0.0;
    }
    let local = elapsed.max(0.0) % (duration * 2.0);
    let leg = if local < duration { local / duration } else { 2.0 - local / duration };
    CubicBezier::EASE_IN_OUT.progress(leg)
}

fn with_alpha(color: Hsla, alpha: f32) -> Hsla {
    Hsla { a: alpha, ..color }
}

/// reduce motion and low power each turn off the breathing and the wake flutter and sparks
/// a still picture stays with the same info
pub enum MapPinMotion {}

impl MapPinMotion {
    const ENDLESS_LOOPS: bool = false;

    pub fn is_allowed(cx: &App) -> bool {
        Self::ENDLESS_LOOPS && !reduce_motion(cx) && !system_low_power_mode()
    }

    /// everything that can flip is_allowed plus foregrounding
    /// the system drops every running layer animation in the background
    pub fn triggers() -> Vec<RiseNotificationName> {
        vec![
            UIAccessibility::REDUCE_MOTION_STATUS_DID_CHANGE_NOTIFICATION,
            RiseNotificationName::new(POWER_STATE_DID_CHANGE),
            UIApplication::WILL_ENTER_FOREGROUND_NOTIFICATION,
        ]
    }
}

/// pin size per zoom
/// full size from street to city, further out they shrink and tile and battery drop out
pub enum MapPinZoom {}

impl MapPinZoom {
    const FULL_SIZE_ZOOM: f64 = 13.0;
    const SMALL_SIZE_ZOOM: f64 = 9.5;
    const SMALL_SCALE: f32 = 0.68;
    const DETAIL_ZOOM: f64 = 12.0;

    pub fn scale(zoom: f64) -> f32 {
        let t = (zoom - Self::SMALL_SIZE_ZOOM) / (Self::FULL_SIZE_ZOOM - Self::SMALL_SIZE_ZOOM);
        Self::SMALL_SCALE + (1.0 - Self::SMALL_SCALE) * t.clamp(0.0, 1.0) as f32
    }

    pub fn shows_detail(zoom: f64) -> bool {
        zoom >= Self::DETAIL_ZOOM
    }
}

/// rotation that always takes the short way round
/// angle is kept unwrapped so 350 to 10 degrees is 20 forward not a full spin back
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MapPinAngle {
    unwrapped: f64,
    is_set: bool,
}

impl MapPinAngle {
    pub fn advance(&mut self, to_degrees: f64) -> f32 {
        if self.is_set {
            self.unwrapped += Self::shortest(to_degrees - self.unwrapped);
        } else {
            self.unwrapped = to_degrees;
            self.is_set = true;
        }
        (self.unwrapped * PI / 180.0) as f32
    }

    pub fn shortest(degrees: f64) -> f64 {
        let mut delta = degrees % 360.0;
        if delta > 180.0 {
            delta -= 360.0;
        }
        if delta < -180.0 {
            delta += 360.0;
        }
        delta
    }

    /// layer is the rotation the decoration paints with, in radians
    pub fn apply(radians: f32, layer: &mut Animated<f32>, animated: bool, now: MotionTime) {
        if animated {
            let shown = layer.value(now);
            let delta = Self::shortest(f64::from(radians - shown) * 180.0 / PI) * PI / 180.0;
            layer.jump(radians - delta as f32);
            layer.set(radians, ease_out(0.22), now, false);
        } else {
            layer.jump(radians);
        }
    }
}

/// heading beam, accent color fading with distance and wider when the compass is less sure
#[derive(Clone, Debug)]
pub struct MapPinConeLayer {
    color: Option<Hsla>,
    angle: MapPinAngle,
    rotation: Animated<f32>,
    half_width: f64,
    wedge: Animated<f32>,
    is_shown: bool,
    opacity: Animated<f32>,
}

/// what the cone paints at one frame time
#[derive(Clone, Copy, Debug)]
pub struct MapPinConeFrame {
    color: Hsla,
    rotation: f32,
    half_width_degrees: f32,
    opacity: f32,
}

impl Default for MapPinConeLayer {
    fn default() -> Self {
        Self::new()
    }
}

impl MapPinConeLayer {
    const REACH: f32 = 78.0;
    const NARROWEST: f64 = 24.0;
    const WIDEST: f64 = 56.0;
    const WIDTH_STEP: f64 = 4.0;
    const BANDS: usize = 12;

    pub fn new() -> Self {
        Self {
            color: None,
            angle: MapPinAngle::default(),
            rotation: Animated::new(0.0),
            half_width: 0.0,
            wedge: Animated::new(0.0),
            is_shown: false,
            opacity: Animated::new(0.0),
        }
    }

    /// side of the square the beam is painted in, before the pin scale
    pub fn side() -> f32 {
        Self::REACH * 2.0
    }

    pub fn set_color(&mut self, color: Hsla) {
        self.color = Some(color);
    }

    /// screen degrees are relative to the top of the screen, compass heading minus map rotation
    pub fn show(&mut self, screen_degrees: f64, accuracy_degrees: f64, animated: bool, now: MotionTime) {
        let clamped = accuracy_degrees.clamp(Self::NARROWEST, Self::WIDEST);
        let width = (clamped / Self::WIDTH_STEP).round() * Self::WIDTH_STEP;
        if width != self.half_width {
            let had_path = self.half_width > 0.0;
            self.half_width = width;
            if animated && self.is_shown && had_path {
                self.wedge.set(width as f32, linear(0.3), now, false);
            } else {
                self.wedge.jump(width as f32);
            }
        }
        let radians = self.angle.advance(screen_degrees);
        MapPinAngle::apply(radians, &mut self.rotation, animated && self.is_shown, now);
        self.set_shown(true, animated, now);
    }

    pub fn hide(&mut self, animated: bool, now: MotionTime) {
        self.set_shown(false, animated, now);
    }

    fn set_shown(&mut self, shown: bool, animated: bool, now: MotionTime) {
        if shown == self.is_shown {
            return;
        }
        self.is_shown = shown;
        let target = if shown { 1.0 } else { 0.0 };
        if animated {
            self.opacity.set(target, implicit_transaction(0.3), now, false);
        } else {
            self.opacity.jump(target);
        }
    }

    pub fn is_running(&self, now: MotionTime) -> bool {
        self.opacity.is_running(now) || self.rotation.is_running(now) || self.wedge.is_running(now)
    }

    pub fn frame(&self, now: MotionTime) -> Option<MapPinConeFrame> {
        let color = self.color?;
        let opacity = self.opacity.value(now);
        if opacity <= 0.0 {
            return None;
        }
        Some(MapPinConeFrame {
            color,
            rotation: self.rotation.value(now),
            half_width_degrees: self.wedge.value(now),
            opacity,
        })
    }

    fn beam_alpha(fraction: f32) -> f32 {
        if fraction <= 0.5 {
            0.62 + (0.26 - 0.62) * (fraction / 0.5)
        } else {
            0.26 * (1.0 - (fraction - 0.5) / 0.5)
        }
    }

    /// paints the wedge around centre with every length times scale
    pub fn paint(frame: MapPinConeFrame, centre: Point<Pixels>, scale: f32, window: &mut Window) {
        let half = frame.half_width_degrees.to_radians();
        let rotation = frame.rotation;
        let ray = |radius: f32, angle: f32| -> Point<Pixels> {
            let screen = angle + rotation - std::f32::consts::FRAC_PI_2;
            point(
                centre.x + px(radius * scale * screen.cos()),
                centre.y + px(radius * scale * screen.sin()),
            )
        };
        for band in 0..Self::BANDS {
            let inner = Self::REACH * band as f32 / Self::BANDS as f32;
            let outer = Self::REACH * (band + 1) as f32 / Self::BANDS as f32;
            let middle = (band as f32 + 0.5) / Self::BANDS as f32;
            let alpha = Self::beam_alpha(middle) * frame.opacity;
            if alpha <= 0.0 {
                continue;
            }
            let mut builder = PathBuilder::fill();
            builder.move_to(ray(inner, -half));
            builder.line_to(ray(outer, -half));
            let radii = point(px(outer * scale), px(outer * scale));
            builder.arc_to(radii, px(0.0), false, true, ray(outer, 0.0));
            builder.arc_to(radii, px(0.0), false, true, ray(outer, half));
            builder.line_to(ray(inner, half));
            if inner > 0.0 {
                let inner_radii = point(px(inner * scale), px(inner * scale));
                builder.arc_to(inner_radii, px(0.0), false, false, ray(inner, 0.0));
                builder.arc_to(inner_radii, px(0.0), false, false, ray(inner, -half));
            }
            builder.close();
            if let Ok(path) = builder.build() {
                window.paint_path(path, with_alpha(frame.color, alpha));
            }
        }
    }
}

/// one spark of the wake at a frame time, in the trail's unrotated space
#[derive(Clone, Copy, Debug)]
struct MapPinSpark {
    x: f32,
    y: f32,
    radius: f32,
    alpha: f32,
}

/// the flame's three parameters
/// outline is linear in them so a morph between outlines is a morph of these
#[derive(Clone, Copy, Debug, PartialEq)]
struct MapPinFlame {
    half_width: f32,
    length: f32,
    sway: f32,
}

impl MapPinFlame {
    fn lerp(self, to: Self, t: f32) -> Self {
        Self {
            half_width: self.half_width + (to.half_width - self.half_width) * t,
            length: self.length + (to.length - self.length) * t,
            sway: self.sway + (to.sway - self.sway) * t,
        }
    }
}

/// what the wake paints at one frame time
#[derive(Clone, Debug)]
pub struct MapPinTrailFrame {
    color: Hsla,
    bright: Hsla,
    rotation: f32,
    stretch: f32,
    opacity: f32,
    outer: MapPinFlame,
    core: MapPinFlame,
    sparks: Vec<MapPinSpark>,
}

/// wake behind a moving pin
/// wide soft flame and bright core, each a gradient masked by a swaying shape
/// plus drifting sparks and with decoration off a still gradient
#[derive(Clone, Debug)]
pub struct MapPinTrailLayer {
    angle: MapPinAngle,
    rotation: Animated<f32>,
    stretch: Animated<f32>,
    opacity: Animated<f32>,
    base_half_width: f32,
    is_shown: bool,
    is_animating: bool,
    shown_color: Option<Hsla>,
    flutter_started_at: Option<MotionTime>,
    sparks_started_at: Option<MotionTime>,
}

impl MapPinTrailLayer {
    const REFERENCE_LENGTH: f32 = 60.0;
    const SIDE: f32 = 156.0;
    const OUTER_FLUTTER: f64 = 1.15;
    const CORE_FLUTTER: f64 = 0.8;
    const SPARK_SIDE: f32 = 10.0;
    const SPARK_BIRTH_RATE: f64 = 5.0;
    const SPARK_LIFETIME: f32 = 1.1;
    const SPARK_LIFETIME_RANGE: f32 = 0.3;
    const SPARK_VELOCITY: f32 = 46.0;
    const SPARK_VELOCITY_RANGE: f32 = 14.0;
    const SPARK_EMISSION_RANGE: f32 = std::f32::consts::PI / 9.0;
    const SPARK_SCALE: f32 = 0.5;
    const SPARK_SCALE_RANGE: f32 = 0.2;
    const SPARK_SCALE_SPEED: f32 = -0.28;
    const SPARK_ALPHA_SPEED: f32 = -0.85;

    pub fn new(base_half_width: f32) -> Self {
        Self {
            angle: MapPinAngle::default(),
            rotation: Animated::new(0.0),
            stretch: Animated::new(1.0),
            opacity: Animated::new(0.0),
            base_half_width,
            is_shown: false,
            is_animating: false,
            shown_color: None,
            flutter_started_at: None,
            sparks_started_at: None,
        }
    }

    /// side of the square the wake is painted in, before the pin scale
    pub fn side() -> f32 {
        Self::SIDE
    }

    pub fn set_color(&mut self, color: Hsla, now: MotionTime) {
        if self.shown_color == Some(color) {
            return;
        }
        self.shown_color = Some(color);
        if self.sparks_started_at.is_some() {
            self.sparks_started_at = Some(now);
        }
    }

    /// screen degrees are relative to the top of the screen
    pub fn show(&mut self, trail: &MapPinTrail, screen_degrees: f64, animated: bool, now: MotionTime, cx: &App) {
        let scale = trail.length as f32 / Self::REFERENCE_LENGTH;
        if animated && self.is_shown {
            self.stretch.set(scale, implicit_transaction(0.6), now, false);
        } else {
            self.stretch.jump(scale);
        }
        let radians = self.angle.advance(screen_degrees);
        MapPinAngle::apply(radians, &mut self.rotation, animated && self.is_shown, now);
        self.set_shown(true, animated, now);
        self.refresh_motion(now, cx);
    }

    /// map turned under the pin so the wake turns with it, no animation
    pub fn turn(&mut self, screen_degrees: f64, now: MotionTime) {
        if !self.is_shown {
            return;
        }
        let radians = self.angle.advance(screen_degrees);
        MapPinAngle::apply(radians, &mut self.rotation, false, now);
    }

    pub fn hide(&mut self, animated: bool, now: MotionTime, cx: &App) {
        self.set_shown(false, animated, now);
        self.refresh_motion(now, cx);
    }

    pub fn refresh_motion(&mut self, now: MotionTime, cx: &App) {
        let wanted = self.is_shown && MapPinMotion::is_allowed(cx);
        if wanted && self.is_animating && self.flutter_started_at.is_some() {
            return;
        }
        self.is_animating = wanted;
        self.flutter_started_at = None;
        self.sparks_started_at = None;
        if !wanted {
            return;
        }
        self.flutter_started_at = Some(now);
        self.sparks_started_at = Some(now);
    }

    fn set_shown(&mut self, shown: bool, animated: bool, now: MotionTime) {
        if shown == self.is_shown {
            return;
        }
        self.is_shown = shown;
        let target = if shown { 1.0 } else { 0.0 };
        if animated {
            self.opacity.set(target, implicit_transaction(0.35), now, false);
        } else {
            self.opacity.jump(target);
        }
    }

    /// flutter and sparks run as long as the wake is shown with motion allowed
    pub fn is_running(&self, now: MotionTime) -> bool {
        self.is_animating
            || self.opacity.is_running(now)
            || self.rotation.is_running(now)
            || self.stretch.is_running(now)
    }

    fn flutter(&self, from: MapPinFlame, to: MapPinFlame, duration: f64, now: MotionTime) -> MapPinFlame {
        match self.flutter_started_at {
            Some(started) => {
                let elapsed = now.saturating_sub(started).as_secs_f64();
                from.lerp(to, autoreversing_progress(elapsed, duration) as f32)
            }
            None => from,
        }
    }

    pub fn frame(&self, now: MotionTime) -> Option<MapPinTrailFrame> {
        let color = self.shown_color?;
        let opacity = self.opacity.value(now);
        if opacity <= 0.0 {
            return None;
        }
        let w = self.base_half_width;
        let length = Self::REFERENCE_LENGTH;
        let outer = self.flutter(
            MapPinFlame { half_width: w, length, sway: w * 0.24 },
            MapPinFlame { half_width: w * 1.04, length: length * 1.08, sway: -w * 0.24 },
            Self::OUTER_FLUTTER,
            now,
        );
        let core = self.flutter(
            MapPinFlame { half_width: w * 0.56, length: length * 0.64, sway: -w * 0.2 },
            MapPinFlame { half_width: w * 0.5, length: length * 0.72, sway: w * 0.2 },
            Self::CORE_FLUTTER,
            now,
        );
        let sparks = match self.sparks_started_at {
            Some(started) => self.sparks(now.saturating_sub(started).as_secs_f64()),
            None => Vec::new(),
        };
        Some(MapPinTrailFrame {
            color,
            bright: Self::lightened(color),
            rotation: self.rotation.value(now),
            stretch: self.stretch.value(now),
            opacity,
            outer,
            core,
            sparks,
        })
    }

    /// fake random spread for the emitter, one value in 0..1 per draw
    fn spread(seed: u64, draw: u64) -> f32 {
        let mut z = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(draw.wrapping_mul(0xBF58_476D_1CE4_E5B9));
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^= z >> 31;
        (z >> 40) as f32 / (1u64 << 24) as f32
    }

    /// live sparks elapsed seconds after the emitter started
    /// five a second from a line across the pin
    /// drifting up the wake and shrinking and fading
    fn sparks(&self, elapsed: f64) -> Vec<MapPinSpark> {
        let oldest_life = f64::from(Self::SPARK_LIFETIME + Self::SPARK_LIFETIME_RANGE);
        let newest = (elapsed * Self::SPARK_BIRTH_RATE).floor().max(0.0) as u64;
        let first = ((elapsed - oldest_life).max(0.0) * Self::SPARK_BIRTH_RATE).floor() as u64;
        let mut sparks = Vec::new();
        for index in first..=newest {
            let age = (elapsed - index as f64 / Self::SPARK_BIRTH_RATE) as f32;
            if age < 0.0 {
                continue;
            }
            let range = |draw: u64| Self::spread(index, draw) * 2.0 - 1.0;
            let lifetime = Self::SPARK_LIFETIME + Self::SPARK_LIFETIME_RANGE * range(1);
            if age > lifetime {
                continue;
            }
            let alpha = 1.0 + Self::SPARK_ALPHA_SPEED * age;
            let scale = Self::SPARK_SCALE + Self::SPARK_SCALE_RANGE * range(2) + Self::SPARK_SCALE_SPEED * age;
            if alpha <= 0.0 || scale <= 0.0 {
                continue;
            }
            let heading = Self::SPARK_EMISSION_RANGE * range(3);
            let velocity = Self::SPARK_VELOCITY + Self::SPARK_VELOCITY_RANGE * range(4);
            let start_x = self.base_half_width * 0.5 * range(5);
            let start_y = -self.base_half_width * 0.7;
            sparks.push(MapPinSpark {
                x: start_x + heading.sin() * velocity * age,
                y: start_y - heading.cos() * velocity * age,
                radius: Self::SPARK_SIDE / 2.0 * scale,
                alpha,
            });
        }
        sparks
    }

    /// paints the wake around centre with every length times scale
    pub fn paint(frame: &MapPinTrailFrame, centre: Point<Pixels>, scale: f32, window: &mut Window) {
        let (sin, cos) = frame.rotation.sin_cos();
        let place = |x: f32, y: f32| -> Point<Pixels> {
            point(centre.x + px((x * cos - y * sin) * scale), centre.y + px((x * sin + y * cos) * scale))
        };
        let angle_degrees = frame.rotation.to_degrees().rem_euclid(360.0);
        for (flame, color, base_alpha) in [(frame.outer, frame.color, 0.88), (frame.core, frame.bright, 0.92)] {
            let path = Self::flame(flame, frame.stretch, &place);
            if let Some(path) = path {
                window.paint_path(
                    path,
                    linear_gradient(
                        angle_degrees,
                        linear_color_stop(with_alpha(color, base_alpha * frame.opacity), 0.0),
                        linear_color_stop(with_alpha(color, 0.0), 1.0),
                    ),
                );
            }
        }
        for spark in &frame.sparks {
            let at = place(spark.x, spark.y);
            for (radius, alpha) in [(spark.radius, 0.35), (spark.radius * 0.5, 1.0)] {
                let side = px(radius * 2.0 * scale);
                let bounds = Bounds::new(point(at.x - side / 2.0, at.y - side / 2.0), size(side, side));
                window.paint_quad(
                    fill(bounds, with_alpha(frame.bright, spark.alpha * alpha * frame.opacity)).corner_radii(side / 2.0),
                );
            }
        }
    }

    /// flame pointing up from the center
    /// pin wide base then a waist and a swell and a tip leaning by sway
    /// all outlines share four curves so one morphs into another
    fn flame(
        flame: MapPinFlame,
        stretch: f32,
        place: &impl Fn(f32, f32) -> Point<Pixels>,
    ) -> Option<gpui::Path<Pixels>> {
        let MapPinFlame { half_width: w, length, sway } = flame;
        let at = |x: f32, y: f32| place(x, -y * length * stretch);
        let mut builder = PathBuilder::fill();
        builder.move_to(at(-w, 0.0));
        builder.cubic_bezier_to(
            at(-w * 0.84 + sway * 0.35, 0.5),
            at(-w * 1.2, 0.2),
            at(-w * 0.66 + sway * 0.2, 0.32),
        );
        builder.cubic_bezier_to(at(sway, 1.0), at(-w * 1.08 + sway * 0.5, 0.72), at(-w * 0.34 + sway, 0.97));
        builder.cubic_bezier_to(
            at(w * 0.84 + sway * 0.35, 0.5),
            at(w * 0.34 + sway, 0.97),
            at(w * 1.08 + sway * 0.5, 0.72),
        );
        builder.cubic_bezier_to(at(w, 0.0), at(w * 0.66 + sway * 0.2, 0.32), at(w * 1.2, 0.2));
        builder.close();
        builder.build().ok()
    }

    fn lightened(color: Hsla) -> Hsla {
        let rgba = color.to_rgb();
        let mix = 0.45;
        Hsla::from(Rgba {
            r: rgba.r + (1.0 - rgba.r) * mix,
            g: rgba.g + (1.0 - rgba.g) * mix,
            b: rgba.b + (1.0 - rgba.b) * mix,
            a: rgba.a,
        })
    }
}

/// tile at a pin's shoulder sized by content
/// emoji alone is a square, duration is two short lines
/// speed is a number over its unit
#[derive(Clone, Debug, Default)]
pub struct MapPinTileView {
    tile: Option<MapPinTile>,
    plate: Option<Hsla>,
    plate_text: Option<Hsla>,
}

impl MapPinTileView {
    const PADDING: f32 = 5.0;
    const EMOJI_SIDE: f32 = 17.0;
    const MIN_HEIGHT: f32 = 22.0;

    pub fn configure(&mut self, tile: Option<&MapPinTile>, palette: &MapPinPalette) -> bool {
        let changed = self.tile.as_ref() != tile
            || self.plate != Some(palette.plate)
            || self.plate_text != Some(palette.plate_text);
        self.plate = Some(palette.plate);
        self.plate_text = Some(palette.plate_text);
        self.tile = tile.cloned();
        changed
    }

    pub fn has_tile(&self) -> bool {
        self.tile.is_some()
    }

    /// the tile at pin scale, or nothing without a tile
    pub fn element(&self, scale: f32, alpha: f32) -> Option<AnyElement> {
        let tile = self.tile.as_ref()?;
        let plate = self.plate?;
        let plate_text = self.plate_text?;
        let s = |value: f32| px(value * scale);
        let has_text = tile.headline.is_some() || tile.caption.is_some();
        let has_glyph = tile.emoji.is_some() || tile.system_image.is_some();

        let mut row = div()
            .flex()
            .flex_row()
            .items_center()
            .min_h(s(Self::MIN_HEIGHT))
            .min_w(s(Self::MIN_HEIGHT))
            .px(s(Self::PADDING))
            .py(s(3.0))
            .gap(s(if has_glyph && has_text { 3.0 } else { 0.0 }))
            .rounded(s(7.0))
            .bg(plate)
            .opacity(alpha);
        if !has_text {
            row = row.justify_center();
        }

        if let Some(symbol) = tile.system_image.as_deref() {
            if let Some(icon) = IconUi::sized(symbol, s(14.0), plate_text, false) {
                row = row.child(div().size(s(Self::EMOJI_SIDE)).flex().items_center().justify_center().child(icon));
            }
        } else if let Some(emoji) = tile.emoji.clone() {
            row = row.child(
                div()
                    .size(s(Self::EMOJI_SIDE))
                    .flex()
                    .items_center()
                    .justify_center()
                    .swiftui_font(Font::system_size(14.0 * scale))
                    .child(emoji),
            );
        }

        if has_text {
            let mut text = div().flex().flex_col();
            if let Some(headline) = tile.headline.clone() {
                text = text.child(
                    div()
                        .swiftui_font(Font::system_weight(11.5 * scale, FontWeight::EXTRA_BOLD))
                        .text_color(plate_text)
                        .child(headline),
                );
            }
            if let Some(caption) = tile.caption.clone() {
                text = text.child(
                    div()
                        .mt(s(-2.0))
                        .swiftui_font(Font::system_weight(8.5 * scale, FontWeight::BOLD))
                        .text_color(with_alpha(plate_text, plate_text.a * 0.86))
                        .child(caption),
                );
            }
            row = row.child(text);
        }
        Some(row.into_any_element())
    }
}

/// the battery pill under the avatar
#[derive(Clone, Debug, Default)]
pub struct MapPinBatteryView {
    shown: Option<(i64, bool)>,
    color: Option<Hsla>,
    text: String,
}

impl MapPinBatteryView {
    const HEIGHT: f32 = 15.0;
    const INSET: f32 = 5.0;
    const GAP: f32 = 2.5;
    const FONT_SIZE: f32 = 9.5;
    const ICON_SIDE: f32 = 9.0;

    pub fn configure(&mut self, level: i64, is_charging: bool, palette: &MapPinPalette) -> bool {
        if self.shown == Some((level, is_charging)) {
            return false;
        }
        self.shown = Some((level, is_charging));
        let is_low = level <= 20 && !is_charging;
        self.color = Some(if is_low { palette.low_battery } else { gpui::white() });
        self.text = RiseFormat::percent(level as f64 / 100.0);
        true
    }

    pub fn height() -> f32 {
        Self::HEIGHT
    }

    /// glyph and digits are two pieces centered on the pill's middle line
    pub fn element(&self, scale: f32, alpha: f32) -> Option<AnyElement> {
        let (level, is_charging) = self.shown?;
        let color = self.color?;
        let s = |value: f32| px(value * scale);
        let mut pill = div()
            .flex()
            .flex_row()
            .items_center()
            .h(s(Self::HEIGHT))
            .px(s(Self::INSET))
            .gap(s(Self::GAP))
            .rounded(s(Self::HEIGHT / 2.0))
            .bg(with_alpha(gpui::black(), 0.62))
            .opacity(alpha);
        if let Some(icon) = IconUi::sized(Self::symbol(level, is_charging), s(Self::ICON_SIDE), color, false) {
            pill = pill.child(icon);
        }
        pill = pill.child(
            div()
                .swiftui_font(Font::system_weight(Self::FONT_SIZE * scale, FontWeight::BOLD).monospaced_digit())
                .text_color(color)
                .child(self.text.clone()),
        );
        Some(pill.into_any_element())
    }

    fn symbol(level: i64, is_charging: bool) -> &'static str {
        if is_charging {
            return "battery.100.bolt";
        }
        match level {
            i64::MIN..=12 => "battery.0",
            13..=37 => "battery.25",
            38..=62 => "battery.50",
            63..=87 => "battery.75",
            _ => "battery.100",
        }
    }
}

/// burst reaction counter, accent pill with white rim at the lower right of the avatar
/// pops in and ticks on every change and fades once the burst rests
#[derive(Clone, Debug)]
pub struct MapPinReactionBadge {
    shown_count: i64,
    color: Option<Hsla>,
    text: String,
    anchor: Point<f32>,
    is_hidden: bool,
    alpha: Animated<f32>,
    scale: Animated<f32>,
}

impl Default for MapPinReactionBadge {
    fn default() -> Self {
        Self {
            shown_count: 0,
            color: None,
            text: String::new(),
            anchor: Point::default(),
            is_hidden: true,
            alpha: Animated::new(1.0),
            scale: Animated::new(1.0),
        }
    }
}

impl MapPinReactionBadge {
    const HEIGHT: f32 = 17.0;
    const RIM: f32 = 1.5;

    pub fn set_color(&mut self, color: Hsla) {
        self.color = Some(color);
    }

    pub fn set(&mut self, count: i64, anchor: Point<f32>, animated: bool, now: MotionTime) {
        if count == self.shown_count {
            return;
        }
        let was_shown = self.shown_count > 0;
        self.shown_count = count;

        if count <= 0 {
            if !(animated && was_shown) {
                self.is_hidden = true;
                self.alpha.jump(1.0);
                self.scale.jump(1.0);
                return;
            }
            self.alpha.set(0.0, ease_in_out(0.22), now, false);
            self.scale.set(0.6, ease_in_out(0.22), now, false);
            return;
        }

        self.text = RiseFormat::compact_count(count);
        self.anchor = anchor;
        self.is_hidden = false;
        self.alpha.jump(1.0);
        if !animated {
            self.scale.jump(1.0);
            return;
        }
        self.scale.jump(if was_shown { 1.22 } else { 0.2 });
        let spec = if was_shown { spring(0.16, 0.6) } else { spring(0.32, 0.55) };
        self.scale.set(1.0, spec, now, false);
    }

    pub fn is_running(&self, now: MotionTime) -> bool {
        !self.is_hidden && (self.alpha.is_running(now) || self.scale.is_running(now))
    }

    /// badge centered on its anchor with every length times scale
    /// nothing once its faded
    pub fn element(&mut self, scale: f32, now: MotionTime) -> Option<AnyElement> {
        if self.is_hidden {
            return None;
        }
        if self.shown_count == 0 && !self.alpha.is_running(now) {
            self.is_hidden = true;
            return None;
        }
        let color = self.color?;
        let pop = self.scale.value(now) * scale;
        let s = |value: f32| px(value * pop);
        let badge = div()
            .flex()
            .items_center()
            .justify_center()
            .h(s(Self::HEIGHT))
            .min_w(s(Self::HEIGHT))
            .px(s(4.5))
            .rounded(s(Self::HEIGHT / 2.0))
            .border(s(Self::RIM))
            .border_color(gpui::white())
            .bg(color)
            .opacity(self.alpha.value(now))
            .swiftui_font(Font::system_weight(10.0 * pop, FontWeight::EXTRA_BOLD).monospaced_digit())
            .text_color(gpui::white())
            .child(self.text.clone());
        Some(
            div()
                .absolute()
                .left(px(self.anchor.x * scale))
                .top(px(self.anchor.y * scale))
                .size(px(0.0))
                .flex()
                .items_center()
                .justify_center()
                .child(badge)
                .into_any_element(),
        )
    }
}
