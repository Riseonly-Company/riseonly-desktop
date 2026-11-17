use std::collections::HashSet;
use std::collections::hash_map::DefaultHasher;
use std::f32::consts::PI;
use std::hash::{Hash, Hasher};
use std::time::Duration;

use gpui::{
    AnyElement, App, Bounds, ContentMask, Context, ElementId, Hsla, IntoElement, ParentElement, Pixels, Styled,
    Window, canvas, div, fill, point, px, size,
};
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::ns_range::NSRange;
use rise_ui::swiftui::motion::CubicBezier;

use crate::core::ui::formatted_text_ui::FormattedTextRenderSeed;
use crate::modules::chat::components::chat::bubble::chat_prepared_text_layout::{
    ChatPreparedTextLayoutCache, ChatTextHitLayoutSeed,
};

/// closed spoiler runs as twinkling dust, text under them is clear
/// opened runs scatter their dust from the center
struct SpoilerDustLayer;

/// one measured spoiler run, its index and rect in the text coordinates
#[derive(Clone, Copy)]
struct SpoilerDustRun {
    index: usize,
    rect: Bounds<Pixels>,
}

#[derive(Default)]
struct SpoilerDustState {
    seed: Option<FormattedTextRenderSeed>,
    display_range: Option<NSRange>,
    closed: HashSet<usize>,
    scatters: Vec<(usize, Duration)>,
}

impl SpoilerDustState {
    fn configure(
        &mut self,
        seed: &FormattedTextRenderSeed,
        display_range: Option<NSRange>,
        closed: &HashSet<usize>,
        now: Duration,
    ) {
        if self.seed.as_ref() != Some(seed) || self.display_range != display_range {
            self.remove_all();
            self.seed = Some(seed.clone());
            self.display_range = display_range;
        }
        let opened: Vec<usize> = self.closed.difference(closed).copied().collect();
        self.closed = closed.clone();
        for index in opened {
            self.scatter(index, now);
        }
        self.scatters
            .retain(|(_, began)| now.saturating_sub(*began).as_secs_f32() < SpoilerDustLayer::SCATTER_DURATION);
    }

    fn scatter(&mut self, index: usize, now: Duration) {
        self.scatters.retain(|(scattering, _)| *scattering != index);
        self.scatters.push((index, now));
    }

    fn remove_all(&mut self) {
        self.closed.clear();
        self.scatters.clear();
    }
}

impl SpoilerDustLayer {
    const LIFETIME: f32 = 1.4;
    const LIFETIME_RANGE: f32 = 0.6;
    const DENSITY: f32 = 0.3;
    const MAX_BIRTH_RATE_PER_RUN: f32 = 900.0;
    const SCATTER_DURATION: f32 = 0.36;
    const VELOCITY: f32 = 3.0;
    const VELOCITY_RANGE: f32 = 5.0;
    const SCALE_RANGE: f32 = 0.4;
    const ALPHA_RANGE: f32 = 0.6;
    const ALPHA_SPEED: f32 = -0.5;
    const DOT_SIDE: f32 = 1.6;
    const SCATTER_SCALE_X: f32 = 1.6;
    const SCATTER_SCALE_Y: f32 = 1.9;

    fn runs(seed: &FormattedTextRenderSeed, display_range: Option<NSRange>, width: Pixels) -> Vec<SpoilerDustRun> {
        let width = f32::from(width);
        if !width.is_finite() || width <= 1.0 {
            return Vec::new();
        }
        let key = ChatTextHitLayoutSeed::new(seed.clone(), width, display_range);
        let marker = ChatPreparedTextLayoutCache::SPOILER_MARKER;
        ChatPreparedTextLayoutCache::request(&key)
            .value_for_immediate_display()
            .hits
            .iter()
            .filter_map(|hit| {
                let index = hit.tag.strip_prefix(marker)?.parse::<usize>().ok()?;
                if hit.rect.size.width <= px(0.5) || hit.rect.size.height <= px(0.5) {
                    return None;
                }
                Some(SpoilerDustRun { index, rect: hit.rect })
            })
            .collect()
    }

    fn paint_run(
        run: SpoilerDustRun,
        origin: gpui::Point<Pixels>,
        color: Hsla,
        now: f32,
        stop: Option<f32>,
        window: &mut Window,
    ) {
        let width = f32::from(run.rect.size.width);
        let height = f32::from(run.rect.size.height);
        let alive = width * height * Self::DENSITY;
        let birth_rate = Self::MAX_BIRTH_RATE_PER_RUN.min(alive / Self::LIFETIME);
        let slots = (birth_rate * Self::LIFETIME).ceil().max(1.0) as u64;
        let center_x = width / 2.0;
        let center_y = height / 2.0;
        let (grow_x, grow_y, fade) = match stop {
            Some(stop) => {
                let progress = ((now - stop) / Self::SCATTER_DURATION).clamp(0.0, 1.0);
                let eased = CubicBezier(0.0, 0.0, 0.58, 1.0).progress(f64::from(progress)) as f32;
                (
                    1.0 + (Self::SCATTER_SCALE_X - 1.0) * eased,
                    1.0 + (Self::SCATTER_SCALE_Y - 1.0) * eased,
                    1.0 - eased,
                )
            }
            None => (1.0, 1.0, 1.0),
        };
        for slot in 0..slots {
            let lifetime = emitter_spread(Self::LIFETIME, Self::LIFETIME_RANGE, unit(run.index, slot, 0, 0));
            let clock = now + unit(run.index, slot, 0, 1) * lifetime;
            let generation = (clock / lifetime).floor();
            let age = clock - generation * lifetime;
            if let Some(stop) = stop
                && now - age > stop
            {
                continue;
            }
            let generation = generation as u64;
            let alpha = (emitter_spread(1.0, Self::ALPHA_RANGE, unit(run.index, slot, generation, 2)) + Self::ALPHA_SPEED * age)
                .clamp(0.0, 1.0)
                * fade;
            if alpha <= 0.0 {
                continue;
            }
            let angle = unit(run.index, slot, generation, 3) * 2.0 * PI;
            let speed = emitter_spread(Self::VELOCITY, Self::VELOCITY_RANGE, unit(run.index, slot, generation, 4));
            let x = unit(run.index, slot, generation, 5) * width + angle.cos() * speed * age;
            let y = unit(run.index, slot, generation, 6) * height + angle.sin() * speed * age;
            let side = Self::DOT_SIDE * emitter_spread(1.0, Self::SCALE_RANGE, unit(run.index, slot, generation, 7));
            let x = center_x + (x - center_x) * grow_x;
            let y = center_y + (y - center_y) * grow_y;
            let mut dot = color;
            dot.a = alpha;
            let bounds = Bounds::new(
                point(origin.x + run.rect.origin.x + px(x - side / 2.0), origin.y + run.rect.origin.y + px(y - side / 2.0)),
                size(px(side), px(side)),
            );
            window.paint_quad(fill(bounds, dot).corner_radii(px(side / 2.0)));
        }
    }
}

/// emitter value with its range, uniform from value minus range to plus range for a unit in 0 to 1
pub fn emitter_spread(value: f32, range: f32, unit: f32) -> f32 {
    value + (2.0 * unit - 1.0) * range
}

/// stable uniform number in 0 to 1 for one particle draw
fn unit(run: usize, slot: u64, generation: u64, channel: u64) -> f32 {
    let mut state = (run as u64)
        .wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ slot.wrapping_mul(0xBF58_476D_1CE4_E5B9)
        ^ generation.wrapping_mul(0x94D0_49BB_1331_11EB)
        ^ channel.wrapping_mul(0xD6E8_FEB8_6659_FD93);
    state ^= state >> 30;
    state = state.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    state ^= state >> 27;
    state = state.wrapping_mul(0x94D0_49BB_1331_11EB);
    state ^= state >> 31;
    (state >> 40) as f32 / (1u64 << 24) as f32
}

pub trait SpoilerDust: IntoElement + Sized {
    /// draws dust under the closed spoilers of seed at this view width
    /// text must be the dust body, which leaves those runs clear
    /// stays attached while there are spoilers so the last opened one still scatters
    fn spoiler_dust(
        self,
        seed: &FormattedTextRenderSeed,
        display_range: Option<NSRange>,
        all: &HashSet<usize>,
        revealed: &HashSet<usize>,
        window: &mut Window,
        cx: &mut App,
    ) -> AnyElement {
        if all.is_empty() {
            return self.into_any_element();
        }
        ChatPreparedTextLayoutCache::install_text_system(cx);
        let closed: HashSet<usize> = all.difference(revealed).copied().collect();
        let mut hasher = DefaultHasher::new();
        seed.hash(&mut hasher);
        display_range.hash(&mut hasher);
        let state = window.use_keyed_state(
            ElementId::NamedInteger("spoiler-dust".into(), hasher.finish()),
            cx,
            |_: &mut Window, _: &mut Context<SpoilerDustState>| SpoilerDustState::default(),
        );
        let now = RiseClock::monotonic();
        state.update(cx, |state, _| state.configure(seed, display_range, &closed, now));
        let scatters = state.read(cx).scatters.clone();
        let seed = seed.clone();
        let mut color = seed.style.base_color;
        color.a = 1.0;

        let layer = canvas(
            move |bounds, _, _| SpoilerDustLayer::runs(&seed, display_range, bounds.size.width),
            move |bounds, runs, window, _| {
                if closed.is_empty() && scatters.is_empty() {
                    return;
                }
                let now = RiseClock::monotonic().as_secs_f32();
                for run in runs {
                    let stop = scatters
                        .iter()
                        .find(|(index, _)| *index == run.index)
                        .map(|(_, began)| began.as_secs_f32());
                    if stop.is_none() && !closed.contains(&run.index) {
                        continue;
                    }
                    let mask = if stop.is_some() {
                        None
                    } else {
                        Some(ContentMask {
                            bounds: Bounds::new(bounds.origin + run.rect.origin, run.rect.size),
                        })
                    };
                    window.with_content_mask(mask, |window| {
                        SpoilerDustLayer::paint_run(run, bounds.origin, color, now, stop, window);
                    });
                }
                window.request_animation_frame();
            },
        )
        .absolute()
        .inset_0();

        div().relative().w_full().child(layer).child(self).into_any_element()
    }
}

impl<E: IntoElement> SpoilerDust for E {}
