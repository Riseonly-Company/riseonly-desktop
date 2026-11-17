use std::cell::RefCell;
use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use futures::StreamExt;
use futures::channel::mpsc;
use gpui::{
    App, AsyncWindowContext, Background, Context, ElementId, EntityId, Hsla, IntoElement, Path, PathBuilder,
    Pixels, Point, RenderOnce, SharedString, Subscription, Task, Window, canvas, point, px,
};
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::rise_app_events::{RiseAppEvents, RiseNotificationName, RiseSubscription, UIAccessibility};
use rise_energy::rise_energy_saving::RiseEnergySavingStore;
use rise_platform::power::POWER_STATE_DID_CHANGE;
use rise_ui::main_text::MainText;
use rise_ui::swiftui::prelude::*;

const POWER_STATE_DID_CHANGE_NOTIFICATION: RiseNotificationName = RiseNotificationName::new(POWER_STATE_DID_CHANGE);

const DOT_COUNT: usize = 3;
const DOT_DIAMETER: f32 = 4.0;
const DOT_SPACING: f32 = 3.0;
const DOTS_WIDTH: f32 = 18.0;
const DOTS_HEIGHT: f32 = 9.0;
const WAVE_DURATION_SECONDS: f32 = 1.5;
const WAVE_DOT_DELAY_SECONDS: f32 = 0.2;
const WAVE_KEY_TIMES: [f32; 4] = [0.0, 0.18, 0.42, 1.0];
const WAVE_OPACITY: [f32; 4] = [0.35, 1.0, 0.35, 0.35];
const WAVE_LIFT: [f32; 4] = [0.0, -2.5, 0.0, 0.0];

#[derive(IntoElement)]
pub struct ChatTypingIndicator {
    id: ElementId,
    text: SharedString,
    color: Hsla,
    font_size: f32,
    trailing: Option<SharedString>,
}

impl ChatTypingIndicator {
    pub fn new(id: impl Into<ElementId>, text: impl Into<SharedString>, color: Hsla) -> Self {
        Self { id: id.into(), text: text.into(), color, font_size: 12.0, trailing: None }
    }

    pub fn font_size(mut self, font_size: f32) -> Self {
        self.font_size = font_size;
        self
    }

    /// shown whole after text, where the line is short the names shorten and never the verb
    pub fn trailing(mut self, trailing: impl Into<Option<SharedString>>) -> Self {
        self.trailing = trailing.into();
        self
    }
}

impl RenderOnce for ChatTypingIndicator {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let names = div()
            .min_w_0()
            .child(MainText::new(self.text).px(self.font_size).color(self.color).number_of_lines(1));
        let trailing = self.trailing.map(|trailing| {
            div().flex_none().child(
                MainText::new(format!(" {trailing}"))
                    .px(self.font_size)
                    .color(self.color)
                    .number_of_lines(1),
            )
        });
        hstack(5.0)
            .child(hstack(0.0).min_w_0().child(names).children(trailing))
            .child(TypingDots { id: self.id, color: self.color })
    }
}

/// inputs of the animation setup that nothing re-renders the row for by itself
struct TypingDotsUIView {
    _observers: Vec<RiseSubscription>,
    _policy_pump: Task<()>,
    _window_activation: Subscription,
}

impl TypingDotsUIView {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let (sender, mut inbox) = mpsc::unbounded::<()>();
        let observers = [
            UIAccessibility::REDUCE_MOTION_STATUS_DID_CHANGE_NOTIFICATION,
            POWER_STATE_DID_CHANGE_NOTIFICATION,
        ]
        .into_iter()
        .map(|name| {
            let sender = sender.clone();
            RiseAppEvents::default_center().observe(name, move |_| {
                let _ = sender.unbounded_send(());
            })
        })
        .collect();
        let policy_pump = cx.spawn(async move |this, cx| {
            while inbox.next().await.is_some() {
                let updated = this.update(cx, |_, cx| cx.notify());
                if updated.is_err() {
                    break;
                }
            }
        });
        let window_activation = cx.observe_window_activation(window, |_, _, cx| cx.notify());
        Self {
            _observers: observers,
            _policy_pump: policy_pump,
            _window_activation: window_activation,
        }
    }

    fn should_animate(window: &Window, cx: &mut App) -> bool {
        let energy_saving = RiseEnergySavingStore::shared(cx);
        window.is_window_active()
            && scene_phase(cx) == ScenePhase::Active
            && !reduce_motion(cx)
            && !energy_saving.read(cx).system_low_power_mode()
    }
}

#[derive(IntoElement)]
struct TypingDots {
    id: ElementId,
    color: Hsla,
}

impl TypingDots {
    fn child_id(&self, suffix: &'static str) -> ElementId {
        ElementId::NamedChild(Arc::new(self.id.clone()), SharedString::new_static(suffix))
    }
}

impl RenderOnce for TypingDots {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let _policy = window.use_keyed_state(self.child_id("policy"), cx, TypingDotsUIView::new);
        let color = self.color;
        let frame = div().relative().flex_none().frame_size(DOTS_WIDTH, DOTS_HEIGHT);
        if !TypingDotsUIView::should_animate(window, cx) {
            return frame.children((0..DOT_COUNT).map(|index| typing_dot(index, color, 1.0, 0.0))).into_any_element();
        }
        let slot = dots_slot(&self.id);
        frame
            .child(
                canvas(|_, _, _| (), move |bounds, (), window, cx| {
                    if !window.content_mask().bounds.intersects(&bounds) {
                        return;
                    }
                    let keys: [u64; DOT_COUNT] = std::array::from_fn(|_| next_dot_swap_key());
                    let view = window.current_view();
                    NEWEST_DOT_KEY.with(|newest| newest.borrow_mut().insert(slot ^ view.as_u64(), keys[DOT_COUNT - 1]));
                    let mut painted = true;
                    for ((path, colour), key) in dot_paths(wave_delta(), color, bounds.origin).into_iter().zip(keys) {
                        match path {
                            Some(path) => window.paint_swappable_path(path, colour, key, bounds.origin),
                            None => painted = false,
                        }
                    }
                    if painted {
                        DotsSwaps { color, keys, view, slot }.schedule(window);
                    }
                })
                .absolute()
                .left_0()
                .top_0()
                .size_full(),
            )
            .into_any_element()
    }
}

struct DotsSwaps {
    color: Hsla,
    keys: [u64; DOT_COUNT],
    view: EntityId,
    slot: u64,
}

impl DotsSwaps {
    fn schedule(self, window: &Window) {
        window.on_next_frame(move |window, cx| self.tick(window, cx));
    }

    fn tick(self, window: &mut Window, cx: &mut App) {
        if !TypingDotsUIView::should_animate(window, cx) {
            return;
        }
        let swaps: Option<Vec<(u64, Path<Pixels>, Background)>> = dot_paths(wave_delta(), self.color, Point::default())
            .into_iter()
            .zip(self.keys)
            .map(|((path, colour), key)| path.map(|path| (key, path, colour.into())))
            .collect();
        if swaps.is_some_and(|swaps| window.swap_paths(swaps)) {
            self.schedule(window);
            return;
        }
        let slot = self.slot ^ self.view.as_u64();
        let last_key = self.keys[DOT_COUNT - 1];
        let repainted = NEWEST_DOT_KEY.with(|newest| {
            let mut newest = newest.borrow_mut();
            match newest.get(&slot) {
                Some(&key) if key > last_key => true,
                _ => {
                    newest.remove(&slot);
                    false
                }
            }
        });
        if repainted {
            return;
        }
        let view = self.view;
        window
            .spawn(cx, async move |cx: &mut AsyncWindowContext| {
                cx.background_executor().timer(FALLBACK_REPAINT_DELAY).await;
                let _ = cx.update(|_, cx| cx.notify(view));
            })
            .detach();
    }
}

const FALLBACK_REPAINT_DELAY: Duration = Duration::from_millis(32);

/// where the wave is now, 0 to 1 of one period
/// every indicator shares the phase
fn wave_delta() -> f32 {
    let period = Duration::from_secs_f32(WAVE_DURATION_SECONDS).as_nanos().max(1);
    (RiseClock::monotonic().as_nanos() % period) as f32 / period as f32
}

/// the three dots of a frame whose corner is at (zero for swaps, laid out from the anchor)
/// each with its colour at delta
fn dot_paths(delta: f32, color: Hsla, at: Point<Pixels>) -> [(Option<Path<Pixels>>, Hsla); DOT_COUNT] {
    let total_width = DOT_DIAMETER * DOT_COUNT as f32 + DOT_SPACING * (DOT_COUNT - 1) as f32;
    let origin_x = (DOTS_WIDTH - total_width) * 0.5;
    let origin_y = (DOTS_HEIGHT - DOT_DIAMETER) * 0.5;
    std::array::from_fn(|index| {
        let (opacity, lift) = typing_wave(dot_progress(index, delta));
        let left = origin_x + index as f32 * (DOT_DIAMETER + DOT_SPACING);
        let top = origin_y + lift;
        (circle_path(point(at.x + px(left), at.y + px(top)), DOT_DIAMETER), color.opacity(opacity))
    })
}

fn circle_path(origin: Point<Pixels>, diameter: f32) -> Option<Path<Pixels>> {
    let radius = diameter / 2.0;
    let radii = point(px(radius), px(radius));
    let mut builder = PathBuilder::fill();
    builder.move_to(point(origin.x, origin.y + px(radius)));
    builder.arc_to(radii, px(0.0), false, true, point(origin.x + px(diameter), origin.y + px(radius)));
    builder.arc_to(radii, px(0.0), false, true, point(origin.x, origin.y + px(radius)));
    builder.close();
    builder.build().ok()
}

fn dots_slot(id: &ElementId) -> u64 {
    let mut hasher = DefaultHasher::new();
    id.hash(&mut hasher);
    hasher.finish()
}

static NEXT_DOT_SWAP_KEY: AtomicU64 = AtomicU64::new((1 << 63) | (1 << 62) | (1 << 61));

fn next_dot_swap_key() -> u64 {
    NEXT_DOT_SWAP_KEY.fetch_add(1, Ordering::Relaxed)
}

thread_local! {
    static NEWEST_DOT_KEY: RefCell<HashMap<u64, u64>> = RefCell::new(HashMap::new());
}

fn typing_dot(index: usize, color: Hsla, opacity: f32, lift: f32) -> Div {
    let total_width = DOT_DIAMETER * DOT_COUNT as f32 + DOT_SPACING * (DOT_COUNT - 1) as f32;
    let origin_x = (DOTS_WIDTH - total_width) * 0.5;
    let origin_y = (DOTS_HEIGHT - DOT_DIAMETER) * 0.5;
    div()
        .absolute()
        .left(px(origin_x + index as f32 * (DOT_DIAMETER + DOT_SPACING)))
        .top(px(origin_y + lift))
        .frame_size(DOT_DIAMETER, DOT_DIAMETER)
        .circle()
        .bg(color)
        .opacity(opacity)
}

fn dot_progress(index: usize, delta: f32) -> f32 {
    let elapsed = delta * WAVE_DURATION_SECONDS - index as f32 * WAVE_DOT_DELAY_SECONDS;
    elapsed.rem_euclid(WAVE_DURATION_SECONDS) / WAVE_DURATION_SECONDS
}

fn typing_wave(progress: f32) -> (f32, f32) {
    for segment in 0..WAVE_KEY_TIMES.len() - 1 {
        let start = WAVE_KEY_TIMES[segment];
        let end = WAVE_KEY_TIMES[segment + 1];
        if progress <= end {
            let local = ((progress - start) / (end - start)).clamp(0.0, 1.0);
            return (
                WAVE_OPACITY[segment] + (WAVE_OPACITY[segment + 1] - WAVE_OPACITY[segment]) * local,
                WAVE_LIFT[segment] + (WAVE_LIFT[segment + 1] - WAVE_LIFT[segment]) * local,
            );
        }
    }
    (WAVE_OPACITY[WAVE_OPACITY.len() - 1], WAVE_LIFT[WAVE_LIFT.len() - 1])
}
