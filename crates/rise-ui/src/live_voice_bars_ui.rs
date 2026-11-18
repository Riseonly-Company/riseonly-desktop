use std::cell::RefCell;
use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use gpui::{
    App, AsyncWindowContext, Background, Bounds, ElementId, EntityId, Hsla, IntoElement, Path, PathBuilder, Pixels,
    Point, RenderOnce, Size, Styled, Window, canvas, point, px, size,
};
use rise_core::foundation::foundation_date::RiseClock;

use crate::frame_request::{CAPPED_TICK_INTERVAL, is_shown_on_screen};
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

const TICKS_BETWEEN_REPAINTS: u32 = 30;
const HIDDEN_RECHECK_DELAY: Duration = Duration::from_millis(500);

static NEXT_BAR_SWAP_KEY: AtomicU64 = AtomicU64::new(0x7 << 60);

thread_local! {
    static NEWEST_BAR_KEY: RefCell<HashMap<u64, u64>> = RefCell::new(HashMap::new());
}

fn next_bar_swap_key() -> u64 {
    NEXT_BAR_SWAP_KEY.fetch_add(1, Ordering::Relaxed)
}

fn bars_slot(id: &ElementId) -> u64 {
    let mut hasher = DefaultHasher::new();
    id.hash(&mut hasher);
    hasher.finish()
}

fn bar_path(frame: Bounds<Pixels>) -> Option<Path<Pixels>> {
    let radius = (f32::from(frame.size.width) * 0.5).max(0.0);
    let radii = point(px(radius), px(radius));
    let left = frame.origin.x;
    let right = frame.origin.x + frame.size.width;
    let top = frame.origin.y + px(radius);
    let bottom = (frame.origin.y + frame.size.height - px(radius)).max(top);
    let mut builder = PathBuilder::fill();
    builder.move_to(point(left, top));
    builder.arc_to(radii, px(0.0), false, true, point(right, top));
    builder.line_to(point(right, bottom));
    builder.arc_to(radii, px(0.0), false, true, point(left, bottom));
    builder.close();
    builder.build().ok()
}

fn bar_paths(bounds: Bounds<Pixels>, elapsed: Duration) -> [Option<Path<Pixels>>; 3] {
    let frames = bar_frames(bounds);
    std::array::from_fn(|index| {
        let frame = frames[index];
        let height = f32::from(frame.size.height) * bar_scale(index, elapsed);
        bar_path(Bounds::new(
            point(frame.origin.x, frame.center().y - px(height / 2.0)),
            size(frame.size.width, px(height)),
        ))
    })
}

struct LiveVoiceBarsInstall {
    base_time: Duration,
}

struct LiveVoiceBarsSwaps {
    color: Hsla,
    keys: [u64; 3],
    view: EntityId,
    slot: u64,
    base_time: Duration,
    size: Size<Pixels>,
    ticks_left: u32,
}

impl LiveVoiceBarsSwaps {
    fn schedule(self, window: &Window, cx: &App) {
        window
            .spawn(cx, async move |cx: &mut AsyncWindowContext| {
                cx.background_executor().timer(CAPPED_TICK_INTERVAL).await;
                let _ = cx.update(move |window, _| window.on_next_frame(move |window, cx| self.tick(window, cx)));
            })
            .detach();
    }

    fn tick(mut self, window: &mut Window, cx: &mut App) {
        if self.ticks_left == 0 {
            if self.is_newest() {
                cx.notify(self.view);
            }
            return;
        }
        self.ticks_left -= 1;
        let elapsed = RiseClock::monotonic().saturating_sub(self.base_time);
        let swaps: Option<Vec<(u64, Path<Pixels>, Background)>> =
            bar_paths(Bounds::new(Point::default(), self.size), elapsed)
                .into_iter()
                .zip(self.keys)
                .map(|(path, key)| path.map(|path| (key, path, self.color.into())))
                .collect();
        if swaps.is_some_and(|swaps| window.swap_paths(swaps)) {
            self.schedule(window, cx);
            return;
        }
        if self.is_newest() {
            NEWEST_BAR_KEY.with(|newest| newest.borrow_mut().remove(&(self.slot ^ self.view.as_u64())));
            self.notify_later(CAPPED_TICK_INTERVAL, window, cx);
        }
    }

    fn is_newest(&self) -> bool {
        let last_key = self.keys[self.keys.len() - 1];
        NEWEST_BAR_KEY
            .with(|newest| newest.borrow().get(&(self.slot ^ self.view.as_u64())).is_some_and(|key| *key <= last_key))
    }

    fn notify_later(&self, delay: Duration, window: &Window, cx: &App) {
        let view = self.view;
        window
            .spawn(cx, async move |cx: &mut AsyncWindowContext| {
                cx.background_executor().timer(delay).await;
                let _ = cx.update(|_, cx| cx.notify(view));
            })
            .detach();
    }
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
        let slot = bars_slot(&self.id);
        let install =
            window.use_keyed_state(self.id, cx, |_, _| LiveVoiceBarsInstall { base_time: RiseClock::monotonic() });
        let base_time = install.read(cx).base_time;
        let animates = self.height > 0.0;
        let color = self.color;
        canvas(
            |_, _, _| (),
            move |bounds, (), window, cx| {
                let elapsed = RiseClock::monotonic().saturating_sub(base_time);
                let keys: [u64; 3] = std::array::from_fn(|_| next_bar_swap_key());
                let view = window.current_view();
                let mut painted = true;
                for (path, key) in bar_paths(bounds, elapsed).into_iter().zip(keys) {
                    match path {
                        Some(path) => window.paint_swappable_path(path, color, key, bounds.origin),
                        None => painted = false,
                    }
                }
                if !animates || !painted {
                    return;
                }
                NEWEST_BAR_KEY.with(|newest| newest.borrow_mut().insert(slot ^ view.as_u64(), keys[keys.len() - 1]));
                let swaps = LiveVoiceBarsSwaps {
                    color,
                    keys,
                    view,
                    slot,
                    base_time,
                    size: bounds.size,
                    ticks_left: TICKS_BETWEEN_REPAINTS,
                };
                if is_shown_on_screen(bounds, window) {
                    swaps.schedule(window, cx);
                } else {
                    swaps.notify_later(HIDDEN_RECHECK_DELAY, window, cx);
                }
            },
        )
        .w(px(self.width))
        .h(px(self.height))
        .flex_none()
    }
}
