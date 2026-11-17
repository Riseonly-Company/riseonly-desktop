use std::sync::Arc;

use gpui::{Bounds, Corners, Pixels, Point, RenderImage, Size, Window, point, px, size};
use rise_core::foundation::foundation_date::RiseClock;

/// dust a deleted message turns into
/// row breaks into squares of its own pixels, loosening leading to trailing edge
/// drift up and fade while neighbours close the gap, pure in frame time
pub enum RiseManualListRemovalDust {}

#[derive(Clone)]
pub struct RiseManualListRemovalDustSource {
    pub image: Arc<RenderImage>,
    pub scale: f32,
    pub frame: Bounds<Pixels>,
}

impl RiseManualListRemovalDustSource {
    fn image_size(&self) -> Size<f32> {
        let pixels = self.image.size(0);
        let scale = if self.scale > 0.0 { self.scale } else { 1.0 };
        size(pixels.width.0 as f32 / scale, pixels.height.0 as f32 / scale)
    }

    fn has_image(&self) -> bool {
        let pixels = self.image.size(0);
        self.image.frame_count() > 0 && pixels.width.0 > 0 && pixels.height.0 > 0
    }
}

/// dust layers a host view carries above its rows, in the host bounds coordinates
#[derive(Default)]
pub struct RiseManualListRemovalDustHost {
    layers: Vec<RiseManualListRemovalDustLayer>,
}

struct RiseManualListRemovalDustLayer {
    removed_at: f64,
    particles: Vec<RiseManualListRemovalDustParticle>,
}

/// one square, its slice of the row image and the keyframe group that carries it away
struct RiseManualListRemovalDustParticle {
    image: Arc<RenderImage>,
    image_frame: Bounds<Pixels>,
    frame: Bounds<Pixels>,
    begin_time: f64,
    life: f64,
    velocity_x: f32,
    velocity_y: f32,
}

impl RiseManualListRemovalDustParticle {
    /// keyframe at step, offset from the model position plus opacity and scale
    fn keyframe(&self, step: usize) -> (Point<f32>, f32, f32) {
        let local = step as f32 / RiseManualListRemovalDust::KEYFRAME_COUNT as f32;
        let seconds = local * self.life as f32;
        let offset = point(
            self.velocity_x * seconds,
            self.velocity_y * seconds - RiseManualListRemovalDust::LIFT_PER_SECOND_SQUARED * seconds * seconds,
        );
        let remaining = 1.0 - local;
        (offset, remaining * remaining, 1.0 - RiseManualListRemovalDust::SHRINK * local)
    }

    /// linear keyframe group at now, none once it ended and the hidden model shows
    fn presentation(&self, now: f64) -> Option<(Point<f32>, f32, f32)> {
        let elapsed = now - self.begin_time;
        if elapsed >= self.life || self.life <= 0.0 {
            return None;
        }
        let progress = (elapsed / self.life).clamp(0.0, 1.0) as f32;
        let scaled = progress * RiseManualListRemovalDust::KEYFRAME_COUNT as f32;
        let step = (scaled.floor() as usize).min(RiseManualListRemovalDust::KEYFRAME_COUNT - 1);
        let fraction = scaled - step as f32;
        let (from_offset, from_opacity, from_scale) = self.keyframe(step);
        let (to_offset, to_opacity, to_scale) = self.keyframe(step + 1);
        Some((
            point(
                from_offset.x + (to_offset.x - from_offset.x) * fraction,
                from_offset.y + (to_offset.y - from_offset.y) * fraction,
            ),
            from_opacity + (to_opacity - from_opacity) * fraction,
            from_scale + (to_scale - from_scale) * fraction,
        ))
    }

    fn paint(&self, origin: Point<Pixels>, now: f64, window: &mut Window) {
        let Some((offset, opacity, scale)) = self.presentation(now) else {
            return;
        };
        if opacity <= 0.0 || scale <= 0.0 {
            return;
        }
        let center = point(
            origin.x + self.frame.origin.x + self.frame.size.width / 2.0 + px(offset.x),
            origin.y + self.frame.origin.y + self.frame.size.height / 2.0 + px(offset.y),
        );
        let model_center = self.frame.center();
        let place = |rect: Bounds<Pixels>| {
            Bounds::new(
                point(
                    center.x + (rect.origin.x - model_center.x) * scale,
                    center.y + (rect.origin.y - model_center.y) * scale,
                ),
                size(rect.size.width * scale, rect.size.height * scale),
            )
        };
        let bounds = place(self.frame);
        let image_bounds = place(self.image_frame);
        let image = self.image.clone();
        window.with_element_opacity(Some(opacity), |window| {
            window.paint_image(bounds, image_bounds, Corners::default(), image, 0, false).ok();
        });
    }
}

impl RiseManualListRemovalDust {
    pub fn play(host: &mut RiseManualListRemovalDustHost, sources: &[RiseManualListRemovalDustSource], leading_is_left: bool) {
        let usable: Vec<&RiseManualListRemovalDustSource> = sources
            .iter()
            .filter(|source| source.frame.size.width > px(0.0) && source.frame.size.height > px(0.0) && source.has_image())
            .collect();
        if usable.is_empty() {
            return;
        }

        let area = usable.iter().fold(0.0_f32, |total, source| {
            total + f32::from(source.frame.size.width) * f32::from(source.frame.size.height)
        });
        let cell = Self::MIN_CELL.max((area / Self::MAX_PARTICLES).sqrt().ceil());

        let now = RiseClock::monotonic().as_secs_f64();
        host.remove_finished(now);
        let mut particles = Vec::new();
        for source in usable {
            Self::add_particles(source, cell, leading_is_left, &mut particles, now);
        }
        host.layers.push(RiseManualListRemovalDustLayer {
            removed_at: now + Self::TOTAL_SECONDS + 0.05,
            particles,
        });
    }

    fn add_particles(
        source: &RiseManualListRemovalDustSource,
        cell: f32,
        leading_is_left: bool,
        dust_layer: &mut Vec<RiseManualListRemovalDustParticle>,
        start_time: f64,
    ) {
        if !source.has_image() {
            return;
        }
        let image_size = source.image_size();
        let width = f32::from(source.frame.size.width).min(image_size.width);
        let height = f32::from(source.frame.size.height).min(image_size.height);
        if width <= 0.0 || height <= 0.0 {
            return;
        }
        let columns = (width / cell).ceil() as usize;
        let rows = (height / cell).ceil() as usize;
        let drift: f32 = if leading_is_left { 1.0 } else { -1.0 };
        dust_layer.reserve(columns * rows);

        for row in 0..rows {
            for column in 0..columns {
                let x = column as f32 * cell;
                let y = row as f32 * cell;
                let side = cell.min(width - x);
                let tall = cell.min(height - y);
                if side <= 0.0 || tall <= 0.0 {
                    continue;
                }

                let along = x / width;
                let sweep = if leading_is_left { along } else { 1.0 - along };
                let delay =
                    f64::from(sweep) * Self::SWEEP_SECONDS + rand::random_range(0.0..=1.0_f64) * Self::JITTER_SECONDS;
                let life = Self::LIFE_MIN_SECONDS
                    + rand::random_range(0.0..=1.0_f64) * (Self::LIFE_MAX_SECONDS - Self::LIFE_MIN_SECONDS);
                let velocity_x =
                    drift * (rand::random_range(0.0..=1.0_f32) * 2.0 - Self::BACKWARD_SHARE) * Self::DRIFT_PER_SECOND;
                let velocity_y = -(Self::RISE_MIN + rand::random_range(0.0..=1.0_f32)) * Self::RISE_PER_SECOND;

                let frame = Bounds::new(
                    point(source.frame.origin.x + px(x), source.frame.origin.y + px(y)),
                    size(px(side), px(tall)),
                );
                let image_frame = Bounds::new(
                    source.frame.origin,
                    size(px(image_size.width), px(image_size.height)),
                );
                dust_layer.push(RiseManualListRemovalDustParticle {
                    image: source.image.clone(),
                    image_frame,
                    frame,
                    begin_time: start_time + delay,
                    life,
                    velocity_x,
                    velocity_y,
                });
            }
        }
    }

    const MAX_PARTICLES: f32 = 1_500.0;
    const MIN_CELL: f32 = 2.5;
    const KEYFRAME_COUNT: usize = 8;
    const SWEEP_SECONDS: f64 = 0.32;
    const JITTER_SECONDS: f64 = 0.08;
    const LIFE_MIN_SECONDS: f64 = 0.45;
    const LIFE_MAX_SECONDS: f64 = 0.8;
    const BACKWARD_SHARE: f32 = 0.6;
    const DRIFT_PER_SECOND: f32 = 34.0;
    const RISE_MIN: f32 = 0.35;
    const RISE_PER_SECOND: f32 = 60.0;
    const LIFT_PER_SECOND_SQUARED: f32 = 40.0;
    const SHRINK: f32 = 0.55;
    const TOTAL_SECONDS: f64 = Self::SWEEP_SECONDS + Self::JITTER_SECONDS + Self::LIFE_MAX_SECONDS;
    pub const GAP_HOLD_SECONDS: f64 = Self::SWEEP_SECONDS + Self::JITTER_SECONDS;
}

impl RiseManualListRemovalDustHost {
    /// true while a batch is still on the host, the host keeps asking for frames until then
    pub fn is_running(&self, now: f64) -> bool {
        self.layers.iter().any(|layer| now < layer.removed_at)
    }

    /// paints every batch above the rows, origin is the host top left in the window
    pub fn paint(&mut self, origin: Point<Pixels>, now: f64, window: &mut Window) {
        self.remove_finished(now);
        if self.layers.is_empty() {
            return;
        }
        for layer in &self.layers {
            for particle in &layer.particles {
                particle.paint(origin, now, window);
            }
        }
        window.request_animation_frame();
    }

    fn remove_finished(&mut self, now: f64) {
        self.layers.retain(|layer| now < layer.removed_at);
    }
}
