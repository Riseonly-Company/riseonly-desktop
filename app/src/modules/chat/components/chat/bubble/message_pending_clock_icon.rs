use std::cell::RefCell;
use std::collections::HashMap;
use std::f32::consts::PI;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use gpui::{
    App, AsyncWindowContext, Background, Bounds, EntityId, FillOptions, Hsla, IntoElement, Path, PathBuilder, PathStyle, Pixels, Point,
    RenderOnce, Window, canvas, fill, point, px, size, white,
};
use rise_core::foundation::foundation_date::{RiseClock, TimeInterval};
use rise_ui::frame_request::{CAPPED_TICK_INTERVAL, is_painted_on_screen};
use rise_ui::swiftui::prelude::*;

static NEXT_HAND_SWAP_KEY: AtomicU64 = AtomicU64::new(1 << 63);

fn next_hand_swap_key() -> u64 {
    NEXT_HAND_SWAP_KEY.fetch_add(1, Ordering::Relaxed)
}

thread_local! {
    static NEWEST_HAND_KEY: RefCell<HashMap<EntityId, u64>> = RefCell::new(HashMap::new());
}

/// animated clock of a message still being sent, a ring with two hands turning at their own periods
#[derive(IntoElement)]
pub struct MessagePendingClockIcon {
    size: f32,
    color: Hsla,
    hour_period: TimeInterval,
    minute_period: TimeInterval,
}

impl Default for MessagePendingClockIcon {
    fn default() -> Self {
        Self::new()
    }
}

impl MessagePendingClockIcon {
    /// defaults are size 10 and white
    /// hour hand every 2.25 s and minute hand every 1.125 s
    pub fn new() -> Self {
        Self {
            size: 10.0,
            color: white(),
            hour_period: 2.25,
            minute_period: 1.125,
        }
    }

    pub fn size(mut self, size: f32) -> Self {
        self.size = size;
        self
    }

    pub fn color(mut self, color: Hsla) -> Self {
        self.color = color;
        self
    }

    pub fn hour_period(mut self, hour_period: TimeInterval) -> Self {
        self.hour_period = hour_period;
        self
    }

    pub fn minute_period(mut self, minute_period: TimeInterval) -> Self {
        self.minute_period = minute_period;
        self
    }
}

impl RenderOnce for MessagePendingClockIcon {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let clock = PendingClockLayerView::configure(self.color, self.hour_period, self.minute_period);
        canvas(
            |_, _: &mut Window, _: &mut App| (),
            move |bounds, (), window, _| clock.paint(bounds, window),
        ).frame_size(self.size, self.size)
    }
}

/// clock layer view with ring, two hands and a hub
/// hands turn while its on screen
#[derive(Clone, Copy)]
struct PendingClockLayerView {
    color: Hsla,
    hour_period: TimeInterval,
    minute_period: TimeInterval,
}

impl PendingClockLayerView {
    fn configure(color: Hsla, hour_period: TimeInterval, minute_period: TimeInterval) -> Self {
        Self {
            color,
            hour_period: hour_period.max(0.2),
            minute_period: minute_period.max(0.2),
        }
    }

    /// rotation from 0 to 2 pi over duration, repeating
    fn rotation(duration: TimeInterval) -> f32 {
        let elapsed = RiseClock::monotonic().as_secs_f64();
        ((elapsed % duration) / duration) as f32 * 2.0 * PI
    }

    fn paint(&self, bounds: Bounds<Pixels>, window: &mut Window) {
        let width = f32::from(bounds.size.width);
        let height = f32::from(bounds.size.height);
        let diameter = width.min(height);
        if diameter <= 0.0 {
            return;
        }
        let line_width = (diameter * 0.09).max(0.8);
        let center = point(
            bounds.left() + px(width * 0.5),
            bounds.top() + px(height * 0.5),
        );

        let ring_radius = diameter * 0.5 - line_width * 0.5;
        let mut ring = PathBuilder::stroke(px(line_width));
        ring.move_to(point(center.x + px(ring_radius), center.y));
        ring.arc_to(point(px(ring_radius), px(ring_radius)), px(0.0), false, true, point(center.x - px(ring_radius), center.y));
        ring.arc_to(point(px(ring_radius), px(ring_radius)), px(0.0), false, true, point(center.x + px(ring_radius), center.y));
        ring.close();
        if let Ok(path) = ring.build() {
            window.paint_path(path, self.color.opacity(0.85));
        }

        let hand_keys = [next_hand_swap_key(), next_hand_swap_key()];
        let view = window.current_view();
        NEWEST_HAND_KEY.with(|newest| newest.borrow_mut().insert(view, hand_keys[1]));
        let local_center = point(px(width * 0.5), px(height * 0.5));
        let mut painted = true;
        for (path, key) in self.hand_paths(diameter, center).into_iter().zip(hand_keys) {
            match path {
                Some(path) => window.paint_swappable_path(path, self.color, key, bounds.origin),
                None => painted = false,
            }
        }

        let hub_diameter = (diameter * 0.08).max(0.7);
        let hub = Bounds::new(
            point(center.x - px(hub_diameter * 0.5), center.y - px(hub_diameter * 0.5)),
            size(px(hub_diameter), px(hub_diameter)),
        );
        window.paint_quad(fill(hub, self.color).corner_radii(px(hub_diameter * 0.5)));

        if painted && is_painted_on_screen(bounds, window) {
            let swaps = PendingClockSwaps {
                clock: *self,
                diameter,
                local_center,
                hand_keys,
                view,
                last_swap: RiseClock::monotonic(),
            };
            swaps.schedule(window);
        }
    }

    /// both hands at the current time around center
    /// window point for a paint, the clocks own point for a swap
    fn hand_paths(&self, diameter: f32, center: Point<Pixels>) -> [Option<Path<Pixels>>; 2] {
        let hand_length = diameter * 0.38;
        let hand_width = (diameter * 0.08).max(0.7);
        [self.hour_period, self.minute_period].map(|period| {
            let angle = Self::rotation(period);
            let end = point(
                center.x + px(hand_length * angle.sin()),
                center.y - px(hand_length * angle.cos()),
            );
            hand_path(center, end, hand_width)
        })
    }
}

struct PendingClockSwaps {
    clock: PendingClockLayerView,
    diameter: f32,
    local_center: Point<Pixels>,
    hand_keys: [u64; 2],
    view: EntityId,
    last_swap: Duration,
}

impl PendingClockSwaps {
    fn schedule(self, window: &Window) {
        window.on_next_frame(move |window, cx| self.tick(window, cx));
    }

    fn tick(mut self, window: &mut Window, cx: &mut App) {
        let now = RiseClock::monotonic();
        if now.saturating_sub(self.last_swap) < CAPPED_TICK_INTERVAL {
            self.schedule(window);
            return;
        }
        self.last_swap = now;
        let swaps: Option<Vec<(u64, Path<Pixels>, Background)>> = self
            .clock
            .hand_paths(self.diameter, self.local_center)
            .into_iter()
            .zip(self.hand_keys)
            .map(|(path, key)| path.map(|path| (key, path, self.clock.color.into())))
            .collect();
        if swaps.is_some_and(|swaps| window.swap_paths(swaps)) {
            self.schedule(window);
            return;
        }
        let view = self.view;
        let repainted = NEWEST_HAND_KEY.with(|newest| {
            let mut newest = newest.borrow_mut();
            match newest.get(&view) {
                Some(&key) if key > self.hand_keys[1] => true,
                _ => {
                    newest.remove(&view);
                    false
                }
            }
        });
        if repainted {
            return;
        }
        window
            .spawn(cx, async move |cx: &mut AsyncWindowContext| {
                cx.background_executor().timer(CAPPED_TICK_INTERVAL).await;
                let _ = cx.update(|_, cx| cx.notify(view));
            })
            .detach();
    }
}

fn hand_path(from: Point<Pixels>, to: Point<Pixels>, line_width: f32) -> Option<Path<Pixels>> {
    let dx = f32::from(to.x - from.x);
    let dy = f32::from(to.y - from.y);
    let direction = dy.atan2(dx);
    let half = line_width * 0.5;
    const CAP_STEPS: usize = 8;
    let at = |around: Point<Pixels>, angle: f32| point(around.x + px(half * angle.cos()), around.y + px(half * angle.sin()));
    let mut points = Vec::with_capacity(2 * CAP_STEPS + 2);
    for step in 0..=CAP_STEPS {
        points.push(at(to, direction - PI * 0.5 + PI * step as f32 / CAP_STEPS as f32));
    }
    for step in 0..=CAP_STEPS {
        points.push(at(from, direction + PI * 0.5 + PI * step as f32 / CAP_STEPS as f32));
    }
    let mut builder = PathBuilder::fill().with_style(PathStyle::Fill(FillOptions::non_zero()));
    builder.add_polygon(&points, true);
    builder.build().ok()
}
