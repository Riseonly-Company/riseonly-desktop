use std::cell::RefCell;
use std::collections::HashMap;
use std::f32::consts::TAU;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use gpui::{
    App, AsyncWindowContext, Background, Bounds, Div, EntityId, Global, Hsla, IntoElement, ParentElement, Path,
    PathBuilder, Pixels, Point, RenderOnce, StyleRefinement, Styled, Window, canvas, div, point, px, relative,
};
use rise_core::foundation::foundation_date::RiseClock;
use rise_theme::ui_color_system::UIColor;

pub const SPOKE_COUNT: usize = 8;

pub const ROTATION_PERIOD: Duration = Duration::from_millis(1000);

pub const MIN_SPOKE_OPACITY: f32 = 0.25;

pub const MEDIUM_SIDE: f32 = 20.0;

pub const LARGE_SIDE: f32 = 37.0;

const SPOKE_WIDTH: f32 = 2.0;
const SPOKE_INNER_RADIUS: f32 = 4.5;
const SPOKE_OUTER_RADIUS: f32 = 9.0;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ControlSize {
    Mini,
    Small,
    #[default]
    Regular,
    Large,
}

impl ControlSize {
    /// side of the circular indicator, only medium and large exist so smaller sizes keep medium
    pub fn progress_side(self) -> f32 {
        match self {
            Self::Mini | Self::Small | Self::Regular => MEDIUM_SIDE,
            Self::Large => LARGE_SIDE,
        }
    }
}

/// duration of one step of the bright spoke
pub fn step_duration() -> Duration {
    ROTATION_PERIOD / SPOKE_COUNT as u32
}

/// spoke that is brightest at now, monotonic time
pub fn head_spoke(now: Duration) -> usize {
    let step = step_duration().as_nanos().max(1);
    ((now.as_nanos() / step) % SPOKE_COUNT as u128) as usize
}

/// opacity of a spoke while head is the brightest, spokes further behind are fainter
pub fn spoke_opacity(spoke: usize, head: usize) -> f32 {
    let behind = (head + SPOKE_COUNT - spoke % SPOKE_COUNT) % SPOKE_COUNT;
    let fall = (1.0 - MIN_SPOKE_OPACITY) / (SPOKE_COUNT - 1) as f32;
    1.0 - behind as f32 * fall
}

/// angle of a spoke in radians, clockwise from twelve o clock
pub fn spoke_angle(spoke: usize) -> f32 {
    spoke as f32 * TAU / SPOKE_COUNT as f32
}

pub fn progress_view() -> ProgressView {
    ProgressView { tint: None, control_size: ControlSize::Regular, spoke_scale: 1.0 }
}

#[derive(IntoElement)]
pub struct ProgressView {
    tint: Option<Hsla>,
    control_size: ControlSize,
    spoke_scale: f32,
}

impl ProgressView {
    pub fn tint(mut self, color: Hsla) -> Self {
        self.tint = Some(color);
        self
    }

    pub fn control_size(mut self, size: ControlSize) -> Self {
        self.control_size = size;
        self
    }

    /// scales the spinner spokes around the center, the layout box keeps its size
    pub fn scale_effect(mut self, scale: f32) -> Self {
        self.spoke_scale = scale.max(0.0);
        self
    }
}

/// views that painted clock driven content in the last frame, one bucket per step
#[derive(Default)]
struct PaintClock {
    buckets: Vec<PaintClockBucket>,
}

struct PaintClockBucket {
    step: Duration,
    views: Vec<EntityId>,
    running: bool,
}

impl Global for PaintClock {}

pub fn note_clock_painted(step: Duration, bounds: Bounds<Pixels>, window: &mut Window, cx: &mut App) {
    if !window.content_mask().bounds.intersects(&bounds) {
        return;
    }
    let view = window.current_view();
    let clock = cx.default_global::<PaintClock>();
    let index = match clock.buckets.iter().position(|bucket| bucket.step == step) {
        Some(index) => index,
        None => {
            clock.buckets.push(PaintClockBucket { step, views: Vec::new(), running: false });
            clock.buckets.len() - 1
        }
    };
    let bucket = &mut clock.buckets[index];
    if !bucket.views.contains(&view) {
        bucket.views.push(view);
    }
    if bucket.running {
        return;
    }
    bucket.running = true;
    cx.spawn(async move |cx| {
        loop {
            cx.background_executor().timer(step).await;
            let views = cx.update(|cx| {
                let clock = cx.default_global::<PaintClock>();
                let bucket = clock.buckets.iter_mut().find(|bucket| bucket.step == step)?;
                let views = std::mem::take(&mut bucket.views);
                if views.is_empty() {
                    bucket.running = false;
                    return None;
                }
                Some(views)
            });
            let Some(views) = views else {
                break;
            };
            cx.update(|cx| {
                for view in views {
                    cx.notify(view);
                }
            });
        }
    })
    .detach();
}

impl RenderOnce for ProgressView {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let tint = self.tint.unwrap_or_else(|| crate::theme(cx).text.secondary);
        let side = self.control_size.progress_side();
        let spoke_scale = self.spoke_scale;

        canvas(|_, _, _| (), move |bounds, (), window, cx| {
            if !window.content_mask().bounds.intersects(&bounds) {
                return;
            }
            let head = head_spoke(RiseClock::monotonic());
            let scale = side / MEDIUM_SIDE * spoke_scale;
            let keys: [u64; SPOKE_COUNT] = std::array::from_fn(|_| next_spoke_swap_key());
            let view = window.current_view();
            NEWEST_SPOKE_KEY.with(|newest| newest.borrow_mut().insert(view, keys[SPOKE_COUNT - 1]));
            let mut painted = true;
            for (spoke, (path, key)) in spoke_paths(bounds.center(), scale).into_iter().zip(keys).enumerate() {
                match path {
                    Some(path) => window.paint_swappable_path(path, tint.opacity(spoke_opacity(spoke, head)), key, bounds.origin),
                    None => painted = false,
                }
            }
            if painted && is_painted_on_screen(bounds, window) {
                let local_center = point(bounds.size.width / 2.0, bounds.size.height / 2.0);
                SpinnerSwaps { tint, scale, local_center, keys, view, head }.schedule(window, cx);
            } else {
                note_clock_painted(step_duration(), bounds, window, cx);
            }
        })
        .size(px(side))
        .flex_none()
    }
}

static NEXT_SPOKE_SWAP_KEY: AtomicU64 = AtomicU64::new((1 << 63) | (1 << 62));

fn next_spoke_swap_key() -> u64 {
    NEXT_SPOKE_SWAP_KEY.fetch_add(1, Ordering::Relaxed)
}

thread_local! {
    static NEWEST_SPOKE_KEY: RefCell<HashMap<EntityId, u64>> = RefCell::new(HashMap::new());
}

struct SpinnerSwaps {
    tint: Hsla,
    scale: f32,
    local_center: Point<Pixels>,
    keys: [u64; SPOKE_COUNT],
    view: EntityId,
    head: usize,
}

impl SpinnerSwaps {
    /// waits for the next step boundary off the frame loop, then swaps in a frame callback
    fn schedule(self, window: &Window, cx: &App) {
        let step = step_duration().as_nanos().max(1);
        let into_step = RiseClock::monotonic().as_nanos() % step;
        let wait = Duration::from_nanos((step - into_step) as u64 + 1_000_000);
        window
            .spawn(cx, async move |cx: &mut AsyncWindowContext| {
                cx.background_executor().timer(wait).await;
                let _ = cx.update(move |window, _| window.on_next_frame(move |window, cx| self.tick(window, cx)));
            })
            .detach();
    }

    fn tick(mut self, window: &mut Window, cx: &mut App) {
        let head = head_spoke(RiseClock::monotonic());
        if head == self.head {
            self.schedule(window, cx);
            return;
        }
        self.head = head;
        let swaps: Option<Vec<(u64, Path<Pixels>, Background)>> = spoke_paths(self.local_center, self.scale)
            .into_iter()
            .zip(self.keys)
            .enumerate()
            .map(|(spoke, (path, key))| path.map(|path| (key, path, self.tint.opacity(spoke_opacity(spoke, head)).into())))
            .collect();
        if swaps.is_some_and(|swaps| window.swap_paths(swaps)) {
            self.schedule(window, cx);
            return;
        }
        let view = self.view;
        let last_key = self.keys[SPOKE_COUNT - 1];
        let repainted = NEWEST_SPOKE_KEY.with(|newest| {
            let mut newest = newest.borrow_mut();
            match newest.get(&view) {
                Some(&key) if key > last_key => true,
                _ => {
                    newest.remove(&view);
                    false
                }
            }
        });
        if repainted {
            return;
        }
        let step = step_duration();
        window
            .spawn(cx, async move |cx: &mut AsyncWindowContext| {
                cx.background_executor().timer(step).await;
                let _ = cx.update(|_, cx| cx.notify(view));
            })
            .detach();
    }
}

/// spokes around center, brightest first by spoke_opacity order
fn spoke_paths(center: Point<Pixels>, scale: f32) -> [Option<Path<Pixels>>; SPOKE_COUNT] {
    let radius = SPOKE_WIDTH * scale / 2.0;
    std::array::from_fn(|spoke| {
        let angle = spoke_angle(spoke);
        let direction = (angle.sin(), -angle.cos());
        let inner = offset(center, direction, SPOKE_INNER_RADIUS * scale);
        let outer = offset(center, direction, SPOKE_OUTER_RADIUS * scale);
        capsule_path(inner, outer, direction, radius)
    })
}

fn is_painted_on_screen(bounds: Bounds<Pixels>, window: &Window) -> bool {
    let clip = window.content_mask().bounds;
    bounds.intersects(&clip) && {
        let visible = bounds.intersect(&clip);
        visible.size.width > Pixels::ZERO && visible.size.height > Pixels::ZERO
    }
}

fn offset(from: Point<Pixels>, direction: (f32, f32), length: f32) -> Point<Pixels> {
    point(from.x + px(direction.0 * length), from.y + px(direction.1 * length))
}

fn capsule_path(
    inner: Point<Pixels>,
    outer: Point<Pixels>,
    direction: (f32, f32),
    radius: f32,
) -> Option<gpui::Path<Pixels>> {
    let normal = (-direction.1, direction.0);
    let radii = point(px(radius), px(radius));
    let mut builder = PathBuilder::fill();
    builder.move_to(offset(inner, normal, radius));
    builder.line_to(offset(outer, normal, radius));
    builder.arc_to(radii, px(0.0), false, false, offset(outer, normal, -radius));
    builder.line_to(offset(inner, normal, -radius));
    builder.arc_to(radii, px(0.0), false, false, offset(inner, normal, radius));
    builder.close();
    builder.build().ok()
}

pub const LINEAR_HEIGHT: f32 = 4.0;

/// filled share of the linear bar, value is pinned to 0 to 1 and nan draws an empty bar
pub fn linear_fraction(value: f64) -> f32 {
    if value.is_nan() { 0.0 } else { value.clamp(0.0, 1.0) as f32 }
}

/// linear progress bar with total 1, takes the width its parent offers
/// narrow it with frame_size
pub fn progress_view_value(value: f64) -> LinearProgressView {
    LinearProgressView { value, tint: None, base: div().w_full().h(px(LINEAR_HEIGHT)) }
}

#[derive(IntoElement)]
pub struct LinearProgressView {
    value: f64,
    tint: Option<Hsla>,
    base: Div,
}

impl LinearProgressView {
    /// tint of the filled part, the track keeps the system fill
    pub fn tint(mut self, color: Hsla) -> Self {
        self.tint = Some(color);
        self
    }
}

impl Styled for LinearProgressView {
    fn style(&mut self) -> &mut StyleRefinement {
        self.base.style()
    }
}

impl RenderOnce for LinearProgressView {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let appearance = crate::theme(cx).appearance;
        let tint = self.tint.unwrap_or_else(|| UIColor::system_blue().resolved_color(appearance));
        let track = UIColor::system_fill().resolved_color(appearance);
        let fraction = linear_fraction(self.value);

        self.base.flex_shrink_0().rounded_full().overflow_hidden().bg(track).child(
            div().h_full().w(relative(fraction)).rounded_full().bg(tint),
        )
    }
}
