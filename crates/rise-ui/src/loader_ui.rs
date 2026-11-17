use std::cell::RefCell;
use std::f32::consts::PI;
use std::rc::Rc;
use std::time::Duration;

use futures::FutureExt;
use futures::future::Shared;
use gpui::{
    App, AsyncApp, Bounds, ElementId, Global, Hsla, IntoElement, ParentElement, PathBuilder, Pixels,
    Point, RenderOnce, Styled, Task, Window, canvas, div, fill, hsla, point, px, size,
};
use rise_core::foundation::foundation_date::RiseClock;
use rise_energy::rise_energy_saving::{RiseEnergySavingFeature, RiseEnergySavingStore};
use rise_theme::ThemeStore;

use crate::frame_request::is_shown_on_screen;
use crate::loader_ui_hooks::{LottieAnimation, loader_ui_hooks};
use crate::swiftui::environment::{ScenePhase, reduce_motion, scene_phase};
use crate::swiftui::progress_view::progress_view;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LoaderUiSize {
    Small,
    Large,
    Custom(f32),
}

impl LoaderUiSize {
    pub fn point_size(self) -> f32 {
        match self {
            Self::Small => 18.,
            Self::Large => 28.,
            Self::Custom(value) => value,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LoaderUiType {
    Default,
    Progress,
    Riseonly,
}

#[derive(IntoElement)]
pub struct LoaderUi {
    pub size: LoaderUiSize,
    pub color: Option<Hsla>,
    pub progress: f64,
    pub r#type: LoaderUiType,
    pub thickness: f32,
    pub track_color: Option<Hsla>,
    pub ring_animated: bool,
    pub speed: f64,
    pub gap: f32,
}

impl Default for LoaderUi {
    fn default() -> Self {
        Self::new()
    }
}

impl LoaderUi {
    pub fn new() -> Self {
        Self {
            size: LoaderUiSize::Large,
            color: None,
            progress: 0.1,
            r#type: LoaderUiType::Default,
            thickness: 2.5,
            track_color: None,
            ring_animated: true,
            speed: 3.,
            gap: 3.,
        }
    }

    pub fn size(mut self, size: LoaderUiSize) -> Self {
        self.size = size;
        self
    }

    pub fn color(mut self, color: Option<Hsla>) -> Self {
        self.color = color;
        self
    }

    pub fn progress(mut self, progress: f64) -> Self {
        self.progress = progress;
        self
    }

    pub fn r#type(mut self, kind: LoaderUiType) -> Self {
        self.r#type = kind;
        self
    }

    pub fn thickness(mut self, thickness: f32) -> Self {
        self.thickness = thickness;
        self
    }

    pub fn track_color(mut self, color: Option<Hsla>) -> Self {
        self.track_color = color;
        self
    }

    pub fn ring_animated(mut self, animated: bool) -> Self {
        self.ring_animated = animated;
        self
    }

    pub fn speed(mut self, speed: f64) -> Self {
        self.speed = speed;
        self
    }

    pub fn gap(mut self, gap: f32) -> Self {
        self.gap = gap;
        self
    }

    pub fn spin_period(&self) -> Duration {
        let normalized = self.speed.clamp(1., 10.);
        Duration::from_secs_f64(1.5 - ((normalized - 1.) * 1.2 / 9.))
    }
}

impl RenderOnce for LoaderUi {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let primary = ThemeStore::shared(cx).read(cx).current_theme().primary._100;
        let tint = self.color.unwrap_or(primary);
        let dim = self.size.point_size();
        let framed = div().size(px(dim)).flex().items_center().justify_center().flex_none();

        match self.r#type {
            LoaderUiType::Default => framed
                .child(div().child(progress_view().tint(tint).scale_effect(dim / 20.)))
                .into_any_element(),
            LoaderUiType::Progress => ProgressRingLoader {
                progress: self.progress,
                size: dim,
                line_width: self.thickness,
                color: self.color.unwrap_or_else(|| hsla(0., 0., 1., 1.)),
                track_color: self.track_color.unwrap_or_else(|| hsla(0., 0., 1., 0.28)),
                animated: self.ring_animated,
                period: self.spin_period(),
                gap: self.gap,
            }
            .into_any_element(),
            LoaderUiType::Riseonly => framed.child(RiseonlyBrandLoader).into_any_element(),
        }
    }
}

/// what a brand loader holds while drawn, the decoded animation
struct RiseonlyBrandLoaderState {
    animation: Option<LottieAnimation>,
    _load: Task<()>,
}

#[derive(IntoElement)]
struct RiseonlyBrandLoader;

impl RenderOnce for RiseonlyBrandLoader {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = window.use_keyed_state(ElementId::Name("riseonly_brand_loader".into()), cx, |_, cx| {
            let load = cx.spawn(async move |loader, cx| {
                let loaded = RiseonlyLoaderAnimationCache::animation(cx).await;
                let _ = loader.update(cx, |loader: &mut RiseonlyBrandLoaderState, cx| {
                    loader.animation = loaded;
                    cx.notify();
                });
            });
            RiseonlyBrandLoaderState {
                animation: None,
                _load: load,
            }
        });

        let reduces_effects = RiseEnergySavingStore::shared(cx).read(cx).reduces(RiseEnergySavingFeature::InterfaceEffects);
        let animation = state.read(cx).animation.clone();
        let is_visible = true;
        let should_animate = is_visible
            && scene_phase(cx) == ScenePhase::Active
            && !reduce_motion(cx)
            && !reduces_effects;

        match (animation, loader_ui_hooks()) {
            (Some(animation), Some(hooks)) => {
                let view = (hooks.lottie_view)(&animation, should_animate, window, cx);
                div().size_full().child(view).into_any_element()
            }
            _ => progress_view().into_any_element(),
        }
    }
}

type PendingAnimation = Shared<Task<Option<LottieAnimation>>>;

#[derive(Default)]
struct RiseonlyLoaderAnimationCache {
    cached: Option<LottieAnimation>,
    pending: Option<PendingAnimation>,
    failed: bool,
}

impl Global for RiseonlyLoaderAnimationCache {}

enum CacheLookup {
    Done(Option<LottieAnimation>),
    Wait(PendingAnimation),
    Start(PendingAnimation),
}

impl RiseonlyLoaderAnimationCache {
    async fn animation(cx: &mut AsyncApp) -> Option<LottieAnimation> {
        let lookup = cx.update(|cx| {
            let cache = cx.default_global::<Self>();
            if let Some(cached) = &cache.cached {
                return CacheLookup::Done(Some(cached.clone()));
            }
            if cache.failed {
                return CacheLookup::Done(None);
            }
            if let Some(pending) = &cache.pending {
                return CacheLookup::Wait(pending.clone());
            }
            let decode = cx
                .background_executor()
                .spawn(async move { loader_ui_hooks().and_then(|hooks| (hooks.riseonly_loader_animation)()) })
                .shared();
            cx.default_global::<Self>().pending = Some(decode.clone());
            CacheLookup::Start(decode)
        });
        match lookup {
            CacheLookup::Done(animation) => animation,
            CacheLookup::Wait(pending) => pending.await,
            CacheLookup::Start(pending) => {
                let animation = pending.await;
                cx.update(|cx| {
                    let cache = cx.default_global::<Self>();
                    cache.pending = None;
                    match &animation {
                        Some(animation) => cache.cached = Some(animation.clone()),
                        None => cache.failed = true,
                    }
                });
                animation
            }
        }
    }
}

#[derive(IntoElement)]
struct ProgressRingLoader {
    progress: f64,
    size: f32,
    line_width: f32,
    color: Hsla,
    track_color: Hsla,
    animated: bool,
    period: Duration,
    gap: f32,
}

impl ProgressRingLoader {
    fn clamped(&self) -> f32 {
        ring_trim(self.progress)
    }

    fn ring_diameter(&self) -> f32 {
        ring_diameter(self.size, self.gap, self.line_width)
    }
}

/// points along a circle of radius around center from start through sweep radians
/// clockwise on a y down screen
pub fn arc_points(center: Point<Pixels>, radius: f32, start: f32, sweep: f32) -> Vec<Point<Pixels>> {
    let steps = ((sweep.abs() / (2. * PI)) * 96.).ceil().max(2.) as usize;
    (0..=steps)
        .map(|step| {
            let angle = start + sweep * step as f32 / steps as f32;
            point(center.x + px(radius * angle.cos()), center.y + px(radius * angle.sin()))
        })
        .collect()
}

pub fn stroke_polyline(points: &[Point<Pixels>], line_width: f32, closed: bool, color: Hsla, window: &mut Window) {
    let mut builder = PathBuilder::stroke(px(line_width));
    builder.add_polygon(points, closed);
    if let Ok(path) = builder.build() {
        window.paint_path(path, color);
    }
}

/// round line cap, gpui strokes have butt ends so each end gets a dot of the line width
pub fn round_cap(at: Point<Pixels>, line_width: f32, color: Hsla, window: &mut Window) {
    let half = line_width / 2.;
    let bounds = Bounds::new(point(at.x - px(half), at.y - px(half)), size(px(line_width), px(line_width)));
    window.paint_quad(fill(bounds, color).corner_radii(px(half)));
}

pub fn ring_spin_degrees(now: Duration, period: Duration) -> f32 {
    let period = period.as_secs_f64();
    if period <= 0. {
        return 0.;
    }
    ((now.as_secs_f64() % period) / period * 360.) as f32
}

impl RenderOnce for ProgressRingLoader {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let (animated, period) = (self.animated, self.period);
        let timeline = window.use_state(cx, |_, _| Rc::new(RefCell::new(ProgressRingTimeline::default()))).read(cx).clone();
        let diameter = self.ring_diameter();
        let sweep = 2. * PI * self.clamped();
        let (line_width, color, track_color) = (self.line_width, self.color, self.track_color);
        canvas(
            move |bounds, window, _| {
                if !animated {
                    return 0.;
                }
                let shown = is_shown_on_screen(bounds, window);
                if shown {
                    window.request_animation_frame();
                }
                timeline.borrow_mut().spin_degrees(RiseClock::monotonic().as_secs_f64(), shown, period)
            },
            move |bounds, spin, window, _| {
                let center = bounds.center();
                let radius = diameter / 2.;
                let track = arc_points(center, radius, 0., 2. * PI);
                stroke_polyline(&track, line_width, true, track_color, window);
                let start = (spin - 90.).to_radians();
                let arc = arc_points(center, radius, start, sweep);
                stroke_polyline(&arc, line_width, false, color, window);
                if let (Some(first), Some(last)) = (arc.first(), arc.last()) {
                    round_cap(*first, line_width, color, window);
                    round_cap(*last, line_width, color, window);
                }
            },
        )
        .size(px(self.size))
        .flex_none()
    }
}

#[derive(Default)]
pub struct ProgressRingClock {
    hidden_time: f64,
    last_shown: Option<f64>,
    is_resuming: bool,
}

impl ProgressRingClock {
    pub fn hold(&mut self) {
        self.is_resuming = false;
    }

    pub fn resume(&mut self) {
        self.is_resuming = self.last_shown.is_some();
    }

    pub fn shown_time(&mut self, time: f64) -> f64 {
        if self.is_resuming
            && let Some(last_shown) = self.last_shown
        {
            self.is_resuming = false;
            self.hidden_time = time - last_shown;
        }
        let shown = time - self.hidden_time;
        self.last_shown = Some(shown);
        shown
    }
}

#[derive(Default)]
struct ProgressRingTimeline {
    clock: ProgressRingClock,
    shown: bool,
    last_frame: Option<f64>,
}

impl ProgressRingTimeline {
    const HIDDEN_GAP: f64 = 0.25;

    fn spin_degrees(&mut self, now: f64, shown: bool, period: Duration) -> f32 {
        let came_back = self.last_frame.is_some_and(|last| now - last > Self::HIDDEN_GAP);
        if shown != self.shown || came_back {
            if shown {
                self.clock.resume();
            } else {
                self.clock.hold();
            }
            self.shown = shown;
        }
        self.last_frame = Some(now);
        let time = if shown { self.clock.shown_time(now) } else { self.clock.last_shown.unwrap_or(now) };
        ring_spin_degrees(Duration::from_secs_f64(time.max(0.)), period)
    }
}

/// fraction of the ring the arc covers for progress, between 0.08 and 1
pub fn ring_trim(progress: f64) -> f32 {
    progress.clamp(0.08, 1.) as f32
}

/// ring diameter, at least 8 and size minus two gaps minus the line width
pub fn ring_diameter(size: f32, gap: f32, line_width: f32) -> f32 {
    (size - 2. * gap - line_width).max(8.)
}
