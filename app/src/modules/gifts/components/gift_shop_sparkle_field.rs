use std::f64::consts::PI;
use std::sync::LazyLock;
use std::time::Duration;

use gpui::{
    App, Background, Bounds, ContentMask, Context, Entity, Hsla, IntoElement, PathBuilder, Pixels, Point, RenderOnce,
    Size, Styled, Window, canvas, point, px,
};
use rise_core::foundation::foundation_date::RiseClock;
use rise_energy::rise_energy_saving::{RiseEnergySavingFeature, RiseEnergySavingStore};
use rise_theme::color;
use rise_ui::swiftui::environment::reduce_motion;

/// golden sparkles around the recipient photo at the top of the shop
/// each leaves the photo edge, drifts out twinkling and fades before the field edge
/// seeded so the field is the same every time, frozen under reduce motion and energy saving
#[derive(IntoElement)]
pub struct GiftShopSparkleField {
    pub source_radius: f32,
    pub is_active: bool,
}

impl Default for GiftShopSparkleField {
    fn default() -> Self {
        Self {
            source_radius: 56.0,
            is_active: true,
        }
    }
}

const MINIMUM_INTERVAL: Duration = Duration::from_nanos(1_000_000_000 / 30);

static SPARKLES: LazyLock<Vec<GiftShopSparkle>> = LazyLock::new(|| GiftShopSparkle::field(52, 0x5EED_61F7));

static PALETTE: LazyLock<[Hsla; 4]> =
    LazyLock::new(|| [color("#FFC940"), color("#F5A623"), color("#FFE3A0"), color("#FFFFFF")]);

/// what the swap loop needs between paints, the field as last painted and whether a loop runs
#[derive(Default)]
struct GiftShopSparkleFieldState {
    swaps_running: bool,
    tick_queued: bool,
    loop_generation: u64,
    animates: bool,
    source_radius: f32,
    size: Size<Pixels>,
}

impl GiftShopSparkleField {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn source_radius(mut self, source_radius: f32) -> Self {
        self.source_radius = source_radius;
        self
    }

    pub fn is_active(mut self, is_active: bool) -> Self {
        self.is_active = is_active;
        self
    }

    /// one frame of the field at time seconds, every sparkle painted with its own swap key so
    /// the loop can move and fade it later
    /// a sparkle out of the field or faded leaves a clear copy at the photo edge
    fn paint(source_radius: f32, time: f64, bounds: Bounds<Pixels>, swap_base: u64, window: &mut Window) {
        for (index, sparkle) in SPARKLES.iter().enumerate() {
            let swap_key = Self::swap_key(swap_base, index);
            let Some((path, fill)) = sparkle.frame(source_radius, time, bounds.size, bounds.origin) else {
                continue;
            };
            if fill.a > 0.0 && path.bounds.intersects(&bounds) {
                window.paint_swappable_path(path, fill, swap_key, bounds.origin);
            } else if let Some((resting, _)) = sparkle.frame(source_radius, 0.0, bounds.size, bounds.origin) {
                let resting = if resting.bounds.intersects(&bounds) {
                    Some(resting)
                } else {
                    let center = point(
                        bounds.origin.x + bounds.size.width / 2.0,
                        bounds.origin.y + bounds.size.height / 2.0,
                    );
                    Self::sparkle_path(center, 1.0)
                };
                if let Some(resting) = resting {
                    window.paint_swappable_path(resting, fill.opacity(0.0), swap_key, bounds.origin);
                }
            }
        }
    }

    /// swap key of one sparkle, the field state entity mixed with the index and never zero
    fn swap_key(swap_base: u64, index: usize) -> u64 {
        (swap_base.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ (index as u64 + 1)).max(1)
    }

    fn run_swaps(state: Entity<GiftShopSparkleFieldState>, generation: u64, window: &Window, cx: &App) {
        window
            .spawn(cx, async move |cx| {
                loop {
                    cx.background_executor().timer(MINIMUM_INTERVAL).await;
                    let keeps_running = cx
                        .update(|window, cx| {
                            let field = state.read(cx);
                            let running = field.swaps_running && field.loop_generation == generation;
                            let queued = field.tick_queued;
                            if running && !queued {
                                state.update(cx, |field, _| field.tick_queued = true);
                                let state = state.clone();
                                window.on_next_frame(move |window, cx| {
                                    state.update(cx, |field, _| field.tick_queued = false);
                                    Self::swap_tick(&state, window, cx);
                                });
                            }
                            running
                        })
                        .unwrap_or(false);
                    if !keeps_running {
                        break;
                    }
                }
            })
            .detach();
    }

    fn swap_tick(state: &Entity<GiftShopSparkleFieldState>, window: &mut Window, cx: &mut App) -> bool {
        let field = state.read(cx);
        let (animates, source_radius, size) = (field.animates, field.source_radius, field.size);
        let still_animates = animates
            && !reduce_motion(cx)
            && !RiseEnergySavingStore::snapshot_reduces(RiseEnergySavingFeature::LoopAnimations);
        let swapped = still_animates && {
            let time = RiseClock::monotonic().as_secs_f64();
            let swap_base = state.entity_id().as_u64();
            let swaps: Vec<(u64, gpui::Path<Pixels>, Background)> = SPARKLES
                .iter()
                .enumerate()
                .filter_map(|(index, sparkle)| {
                    let (path, fill) = sparkle.frame(source_radius, time, size, Point::default())?;
                    Some((Self::swap_key(swap_base, index), path, fill.into()))
                })
                .collect();
            window.swap_paths(swaps)
        };
        if !swapped {
            state.update(cx, |field, cx| {
                field.swaps_running = false;
                if animates {
                    cx.notify();
                }
            });
        }
        swapped
    }

    /// four pointed sparkle with concave sides
    fn sparkle_path(center: Point<Pixels>, radius: f32) -> Option<gpui::Path<Pixels>> {
        let pinch = px(radius * 0.16);
        let radius = px(radius);
        let mut builder = PathBuilder::fill();
        builder.move_to(point(center.x, center.y - radius));
        builder.curve_to(point(center.x + radius, center.y), point(center.x + pinch, center.y - pinch));
        builder.curve_to(point(center.x, center.y + radius), point(center.x + pinch, center.y + pinch));
        builder.curve_to(point(center.x - radius, center.y), point(center.x - pinch, center.y + pinch));
        builder.curve_to(point(center.x, center.y - radius), point(center.x - pinch, center.y - pinch));
        builder.close();
        builder.build().ok()
    }

    fn smoothstep(edge0: f64, edge1: f64, value: f64) -> f64 {
        let t = ((value - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    }
}

impl RenderOnce for GiftShopSparkleField {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = window.use_state(cx, |_, _: &mut Context<GiftShopSparkleFieldState>| {
            GiftShopSparkleFieldState::default()
        });
        let animates = self.is_active
            && !reduce_motion(cx)
            && !RiseEnergySavingStore::snapshot_reduces(RiseEnergySavingFeature::LoopAnimations);
        let source_radius = self.source_radius;
        canvas(
            |_, _, _| (),
            move |bounds, _, window, cx| {
                let time = if animates {
                    RiseClock::monotonic().as_secs_f64()
                } else {
                    0.0
                };
                let swap_base = state.entity_id().as_u64();
                window.with_content_mask(Some(ContentMask { bounds }), |window| {
                    Self::paint(source_radius, time, bounds, swap_base, window);
                });
                let started = state.update(cx, |field, _| {
                    field.animates = animates;
                    field.source_radius = source_radius;
                    field.size = bounds.size;
                    if !animates || field.swaps_running {
                        return None;
                    }
                    field.swaps_running = true;
                    field.loop_generation = field.loop_generation.wrapping_add(1);
                    Some(field.loop_generation)
                });
                if let Some(generation) = started {
                    Self::run_swaps(state.clone(), generation, window, cx);
                }
            },
        )
        .size_full()
    }
}

struct GiftShopSparkle {
    angle: f64,
    drift: f64,
    phase: f64,
    size: f64,
    opacity: f64,
    twinkle_speed: f64,
    twinkle_phase: f64,
    color_index: usize,
}

impl GiftShopSparkle {
    /// the sparkle at time seconds in a field of size at origin, its outline and fill
    /// clear once faded below what the eye sees
    fn frame(
        &self,
        source_radius: f32,
        time: f64,
        size: Size<Pixels>,
        origin: Point<Pixels>,
    ) -> Option<(gpui::Path<Pixels>, Hsla)> {
        let width = f64::from(f32::from(size.width));
        let height = f64::from(f32::from(size.height));
        let center_x = f64::from(f32::from(origin.x)) + width / 2.0;
        let center_y = f64::from(f32::from(origin.y)) + height / 2.0;
        let source_radius = f64::from(source_radius);
        let reach = (width * width + height * height).sqrt() / 2.0;
        let progress = (time * self.drift + self.phase) % 1.0;
        let distance = source_radius + 6.0 + progress * (reach - source_radius);
        let x = center_x + self.angle.cos() * distance;
        let y = center_y + self.angle.sin() * distance * 0.82;
        let twinkle = 0.5 + 0.5 * (time * self.twinkle_speed + self.twinkle_phase).sin();
        let fade = GiftShopSparkleField::smoothstep(0.0, 0.12, progress)
            * (1.0 - GiftShopSparkleField::smoothstep(0.62, 1.0, progress));
        let opacity = self.opacity * fade * (0.35 + 0.65 * twinkle);
        let side = self.size * (0.65 + 0.35 * twinkle);
        let mut fill = PALETTE[self.color_index % PALETTE.len()];
        fill.a = if opacity <= 0.02 {
            0.0
        } else {
            fill.a * opacity as f32
        };
        let path = GiftShopSparkleField::sparkle_path(point(px(x as f32), px(y as f32)), (side / 2.0) as f32)?;
        Some((path, fill))
    }

    fn field(count: usize, seed: u64) -> Vec<GiftShopSparkle> {
        let mut random = SplitMix64 { state: seed };
        (0..count)
            .map(|index| {
                let large = index % 9 == 0;
                let angle = random.unit() * 2.0 * PI;
                let drift = 0.035 + random.unit() * 0.045;
                let phase = random.unit();
                let size = if large {
                    11.0 + random.unit() * 5.0
                } else {
                    4.0 + random.unit() * 6.0
                };
                let opacity = 0.55 + random.unit() * 0.45;
                let twinkle_speed = 1.6 + random.unit() * 2.6;
                let twinkle_phase = random.unit() * 2.0 * PI;
                let color_index = (random.unit() * 4.0) as usize;
                GiftShopSparkle {
                    angle,
                    drift,
                    phase,
                    size,
                    opacity,
                    twinkle_speed,
                    twinkle_phase,
                    color_index,
                }
            })
            .collect()
    }
}

struct SplitMix64 {
    state: u64,
}

impl SplitMix64 {
    fn next(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// uniform random in 0 to 1 excluding 1
    fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }
}
