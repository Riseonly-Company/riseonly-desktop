use std::cell::Cell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use gpui::{
    AnyWindowHandle, App, AppContext, AssetSource, Context, Corners, ElementId, Global, IntoElement, ParentElement,
    RenderImage, RenderOnce, SharedString, Styled, Task, Window, canvas, div, px,
};
use image::{Delay, Frame, RgbaImage};
use rise_core::foundation::foundation_date::RiseClock;
use rise_media::lottie::container::to_animation_json;
use rise_media::lottie::raster_policy::{self, SequenceKey};
use rise_media::lottie::sequence::{DecodedFrame, DecodedFrameCache, FrameSequence, LottieRasterizer};
use rise_theme::ui_color_system::UIColor;
use rise_theme::{Appearance, RiseThemeAppearance, ThemeStore};
use smallvec::SmallVec;

use crate::frame_request::is_shown_on_screen;
use crate::swiftui::environment::reduce_motion;

const LOTTIE_AVAILABLE: bool = cfg!(feature = "rlottie");

const SUBDIRECTORIES: [&str; 2] = ["animations/red_panda", "animations"];

const ANIMATION_COUNT_LIMIT: usize = 32;

const PAUSED_PROGRESS: f64 = 0.5;

/// bundled Lottie animation played by name
#[derive(IntoElement, Clone, Debug, PartialEq)]
pub struct LottieView {
    pub name: SharedString,
    pub size: f32,
    pub looping: bool,
    pub speed: f32,
}

impl LottieView {
    pub fn new(name: impl Into<SharedString>) -> Self {
        Self {
            name: name.into(),
            size: 50.0,
            looping: true,
            speed: 1.0,
        }
    }

    pub fn size(mut self, size: f32) -> Self {
        self.size = size;
        self
    }

    pub fn looping(mut self, looping: bool) -> Self {
        self.looping = looping;
        self
    }

    pub fn speed(mut self, speed: f32) -> Self {
        self.speed = speed;
        self
    }

    /// true when name resolves and parses the same way the view plays it
    /// so a caller can show a static fallback
    /// resolved once per name and the parsed model is reused by the view
    pub fn can_play(name: &str, cx: &mut App) -> bool {
        LottieViewAnimationStore::can_play(name, cx)
    }

    pub fn prewarm(name: &str, cx: &mut App) {
        LottieViewAnimationStore::prewarm(name, cx);
    }

    /// same cached model, for a caller that drives playback itself
    pub fn make_player(name: &str, cx: &mut App) -> Option<LottieViewModel> {
        LottieViewAnimationStore::make_animation_view(name, cx)
    }

    /// parsed model if the store already holds it, no parsing
    pub fn cached(name: &str, cx: &mut App) -> Option<LottieViewModel> {
        LottieViewAnimationStore::cached(name, cx)
    }

    /// parsed model, parses off the main thread on a miss
    /// result lands in the shared store before the task ends
    pub fn load(name: &str, cx: &mut App) -> Task<Option<LottieViewModel>> {
        LottieViewAnimationStore::load(name, cx)
    }
}

impl RenderOnce for LottieView {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let side = px(self.size);
        if !LOTTIE_AVAILABLE {
            return div()
                .w(side)
                .h(side)
                .flex_none()
                .rounded(px(20.0))
                .bg(system_gray(cx).opacity(0.3))
                .into_any_element();
        }

        let reduces_motion = reduce_motion(cx);
        let configuration = PlayerConfiguration {
            dimension: raster_policy::dimension(self.size, window.scale_factor()),
            looping: if reduces_motion { false } else { self.looping },
            speed: if reduces_motion { 0.0 } else { self.speed },
        };
        let name = self.name.clone();
        let player = window.use_keyed_state(ElementId::Name(format!("lottie_view/{name}").into()), cx, |_, cx| {
            LottieViewRepresentable::make_ui_view(cx)
        });
        player.update(cx, |player, cx| player.update_ui_view(name.clone(), configuration, cx));

        let mut container = div().w(side).h(side).flex_none().overflow_hidden();
        let (frames, started_at, delay, shown_frame) = {
            let player = player.read(cx);
            (player.frames.clone(), player.started_at, player.delay, player.shown_frame.clone())
        };
        if !frames.is_empty() {
            let frame_count = frames.len();
            let frame_index = timeline_frame_index(
                RiseClock::monotonic().saturating_sub(started_at),
                delay,
                frame_count,
                configuration.looping,
            );
            let image = frames[frame_index].clone();
            let wants_next_frame = frame_count > 1 && (configuration.looping || frame_index + 1 < frame_count);
            let swap_window = window.window_handle();
            let swap_key = (player.entity_id().as_u64() as u32).wrapping_add(1);
            let ticking_player = player.clone();
            container = container.child(
                canvas(
                    move |bounds, window, cx| {
                        if wants_next_frame && is_shown_on_screen(bounds, window) {
                            ticking_player
                                .update(cx, |player, cx| player.ensure_ticking(swap_window, swap_key, cx));
                        }
                    },
                    move |bounds, _, window, _| {
                        shown_frame.set(Some(frame_index));
                        let painted = window.with_image_swap_key(swap_key, |window| {
                            window.paint_image(bounds, bounds, Corners::default(), image, 0, false)
                        });
                        if let Err(error) = painted {
                            tracing::warn!(target: "riseonly", "lottie frame does not paint: {error:?}");
                        }
                    },
                )
                .w(side)
                .h(side),
            );
        }
        container.into_any_element()
    }
}

fn system_gray(cx: &mut App) -> gpui::Hsla {
    let appearance = match ThemeStore::shared(cx).read(cx).resolved_appearance() {
        RiseThemeAppearance::Light => Appearance::Light,
        RiseThemeAppearance::Dark => Appearance::Dark,
    };
    UIColor::system_gray().resolved_color(appearance)
}

/// a parsed animation, its Lottie json proved to open in the rasteriser
#[derive(Clone)]
pub struct LottieViewModel {
    name: SharedString,
    json: Arc<[u8]>,
}

impl std::fmt::Debug for LottieViewModel {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("LottieViewModel")
            .field("name", &self.name)
            .field("json_bytes", &self.json.len())
            .finish()
    }
}

impl LottieViewModel {
    pub fn name(&self) -> &SharedString {
        &self.name
    }

    pub fn json(&self) -> &[u8] {
        &self.json
    }

    pub fn open_rasterizer(&self) -> Option<Box<dyn LottieRasterizer>> {
        open_rasterizer(&self.json, &self.name)
    }
}

#[cfg(feature = "rlottie")]
fn open_rasterizer(json: &[u8], cache_key: &str) -> Option<Box<dyn LottieRasterizer>> {
    match rise_media::lottie::rlottie_backend::RlottieRasterizer::open(json, cache_key) {
        Ok(rasterizer) => Some(Box::new(rasterizer)),
        Err(error) => {
            tracing::warn!(target: "riseonly", "lottie {cache_key} does not open: {error:?}");
            None
        }
    }
}

#[cfg(not(feature = "rlottie"))]
fn open_rasterizer(_json: &[u8], _cache_key: &str) -> Option<Box<dyn LottieRasterizer>> {
    None
}

/// where a name resolved
/// an asset file or nowhere on disk (the desktop bundle has no catalogue so nothing parses)
#[derive(Clone, PartialEq, Eq, Debug)]
enum Source {
    File { path: String },
    Bundled,
}

/// resolves an animation location once per name and keeps the parsed model
/// so a remount (eg a premium badge in every list row) only builds the view
#[derive(Default)]
struct LottieViewAnimationStore {
    sources: HashMap<SharedString, Source>,
    playable: HashMap<SharedString, bool>,
    animations: HashMap<SharedString, Option<LottieViewModel>>,
    recency: VecDeque<SharedString>,
    warming: HashSet<SharedString>,
}

impl Global for LottieViewAnimationStore {}

impl LottieViewAnimationStore {
    fn can_play(name: &str, cx: &mut App) -> bool {
        if let Some(known) = cx.default_global::<Self>().playable.get(name) {
            return *known;
        }
        let result = Self::animation(name, cx).is_some();
        cx.default_global::<Self>().playable.insert(SharedString::from(name.to_owned()), result);
        result
    }

    fn prewarm(name: &str, cx: &mut App) {
        let key = SharedString::from(name.to_owned());
        {
            let store = cx.default_global::<Self>();
            if store.animations.contains_key(&key) || store.warming.contains(&key) {
                return;
            }
            store.warming.insert(key.clone());
        }
        let source = Self::source(&key, cx);
        let assets = cx.asset_source().clone();
        let parsing_key = key.clone();
        let parsing = cx.background_spawn(async move { parse(&parsing_key, &source, &assets) });
        cx.spawn(async move |cx| {
            let parsed = parsing.await;
            cx.update(|cx| {
                let store = cx.default_global::<Self>();
                store.warming.remove(&key);
                if store.animations.contains_key(&key) {
                    return;
                }
                store.playable.insert(key.clone(), parsed.is_some());
                store.insert(key, parsed);
            });
        })
        .detach();
    }

    fn cached(name: &str, cx: &mut App) -> Option<LottieViewModel> {
        let key = SharedString::from(name.to_owned());
        let store = cx.default_global::<Self>();
        let hit = store.animations.get(&key).cloned()?;
        store.touch(&key);
        hit
    }

    fn load(name: &str, cx: &mut App) -> Task<Option<LottieViewModel>> {
        let key = SharedString::from(name.to_owned());
        {
            let store = cx.default_global::<Self>();
            if let Some(hit) = store.animations.get(&key).cloned() {
                store.touch(&key);
                return Task::ready(hit);
            }
        }
        let source = Self::source(&key, cx);
        let assets = cx.asset_source().clone();
        let parsing_key = key.clone();
        let parsing = cx.background_spawn(async move { parse(&parsing_key, &source, &assets) });
        cx.spawn(async move |cx| {
            let parsed = parsing.await;
            cx.update(|cx| {
                let store = cx.default_global::<Self>();
                if let Some(stored) = store.animations.get(&key).cloned() {
                    store.touch(&key);
                    return stored;
                }
                store.playable.insert(key.clone(), parsed.is_some());
                store.insert(key, parsed.clone());
                parsed
            })
        })
    }

    /// cached model a view plays, parsed here on a miss
    fn make_animation_view(name: &str, cx: &mut App) -> Option<LottieViewModel> {
        Self::animation(name, cx)
    }

    fn animation(name: &str, cx: &mut App) -> Option<LottieViewModel> {
        let key = SharedString::from(name.to_owned());
        {
            let store = cx.default_global::<Self>();
            if let Some(hit) = store.animations.get(&key).cloned() {
                store.touch(&key);
                return hit;
            }
        }
        let source = Self::source(&key, cx);
        let assets = cx.asset_source().clone();
        let parsed = parse(&key, &source, &assets);
        cx.default_global::<Self>().insert(key, parsed.clone());
        parsed
    }

    fn source(name: &SharedString, cx: &mut App) -> Source {
        if let Some(cached) = cx.default_global::<Self>().sources.get(name) {
            return cached.clone();
        }
        let assets = cx.asset_source().clone();
        let resolved = candidate_paths(name)
            .into_iter()
            .find(|path| matches!(assets.load(path), Ok(Some(_))))
            .map_or(Source::Bundled, |path| Source::File { path });
        cx.default_global::<Self>().sources.insert(name.clone(), resolved.clone());
        resolved
    }

    fn insert(&mut self, key: SharedString, animation: Option<LottieViewModel>) {
        self.animations.insert(key.clone(), animation);
        self.touch(&key);
        while self.recency.len() > ANIMATION_COUNT_LIMIT {
            if let Some(evicted) = self.recency.pop_front() {
                self.animations.remove(&evicted);
            }
        }
    }

    fn touch(&mut self, key: &SharedString) {
        self.recency.retain(|known| known != key);
        self.recency.push_back(key.clone());
    }
}

/// asset paths a name may resolve to, in search order
fn candidate_paths(name: &str) -> Vec<String> {
    SUBDIRECTORIES
        .iter()
        .map(|directory| format!("{directory}/{name}.json"))
        .chain(std::iter::once(format!("{name}.json")))
        .collect()
}

/// reads and proves one animation, blocking
/// runs on the gpui thread only for a first can_play or mount
/// otherwise on the background executor
fn parse(name: &SharedString, source: &Source, assets: &Arc<dyn AssetSource>) -> Option<LottieViewModel> {
    let Source::File { path } = source else {
        return None;
    };
    let bytes = assets.load(path).ok().flatten()?;
    let json = to_animation_json(&bytes).ok()?;
    let rasterizer = open_rasterizer(&json, name)?;
    if rasterizer.frame_count() == 0 {
        return None;
    }
    Some(LottieViewModel {
        name: name.clone(),
        json: Arc::from(json),
    })
}

#[derive(Clone, Copy, PartialEq, Debug)]
struct PlayerConfiguration {
    dimension: u32,
    looping: bool,
    speed: f32,
}

struct LottieViewRepresentable {
    name: Option<SharedString>,
    configuration: Option<PlayerConfiguration>,
    frames: Vec<Arc<RenderImage>>,
    delay: Duration,
    started_at: Duration,
    finished: bool,
    shown_frame: Rc<Cell<Option<usize>>>,
    swap_window: Option<AnyWindowHandle>,
    swap_key: u32,
    ticking: bool,
    _ticker: Task<()>,
    _work: Task<()>,
    _release: gpui::Subscription,
}

impl LottieViewRepresentable {
    fn make_ui_view(cx: &mut Context<Self>) -> Self {
        let release = cx.on_release(|player, cx| {
            for image in player.frames.drain(..) {
                LottieRasterCache::release_image(image, cx);
            }
        });
        Self {
            name: None,
            configuration: None,
            frames: Vec::new(),
            delay: Duration::ZERO,
            started_at: Duration::ZERO,
            finished: false,
            shown_frame: Rc::new(Cell::new(None)),
            swap_window: None,
            swap_key: 0,
            ticking: false,
            _ticker: Task::ready(()),
            _work: Task::ready(()),
            _release: release,
        }
    }

    fn update_ui_view(&mut self, name: SharedString, configuration: PlayerConfiguration, cx: &mut Context<Self>) {
        let same_name = self.name.as_ref() == Some(&name);
        if same_name && self.configuration == Some(configuration) {
            return;
        }
        let same_raster = same_name
            && self
                .configuration
                .is_some_and(|previous| previous.dimension == configuration.dimension);
        self.name = Some(name.clone());
        self.configuration = Some(configuration);
        if !same_raster {
            self.finished = false;
        }
        if configuration.speed != 0.0 && !configuration.looping && self.finished {
            return;
        }

        let Some(model) = LottieViewAnimationStore::make_animation_view(&name, cx) else {
            self.replace_frames(Vec::new(), Duration::ZERO, cx);
            self._work = Task::ready(());
            return;
        };
        let mode = if configuration.speed == 0.0 {
            RasterMode::Paused
        } else {
            RasterMode::Playing {
                speed: configuration.speed,
            }
        };
        let dimension = configuration.dimension;
        let looping = configuration.looping;
        let raster_key = RasterKey::new(&name, dimension, mode);
        if let Some(raster) = LottieRasterCache::get(&raster_key, cx) {
            self.adopt_frames(raster.frames, raster.delay, cx);
            let animates = self.frames.len() > 1;
            if looping || !animates {
                self._work = Task::ready(());
                return;
            }
            let run = raster.duration;
            self._work = cx.spawn(async move |player, cx| {
                cx.background_executor().timer(run).await;
                let _ = player.update(cx, |player, cx| player.finish_run(cx));
            });
            return;
        }
        let rasterizing = cx.background_spawn(async move { rasterize(&model, dimension, mode) });
        self._work = cx.spawn(async move |player, cx| {
            let Some(rendered) = rasterizing.await else {
                let _ = player.update(cx, |player, cx| player.replace_frames(Vec::new(), Duration::ZERO, cx));
                return;
            };
            cx.update(|cx| LottieRasterCache::insert(raster_key, &rendered, cx));
            let run = rendered.duration;
            let animates = rendered.frames.len() > 1;
            let shown = player.update(cx, |player, cx| player.replace_frames(rendered.frames, rendered.delay, cx));
            if shown.is_err() || looping || !animates {
                return;
            }
            cx.background_executor().timer(run).await;
            let _ = player.update(cx, |player, cx| player.finish_run(cx));
        });
    }

    fn replace_frames(&mut self, frames: Vec<Arc<RenderImage>>, delay: Duration, cx: &mut Context<Self>) {
        let stays_empty = self.frames.is_empty() && frames.is_empty();
        self.adopt_frames(frames, delay, cx);
        if !stays_empty {
            cx.notify();
        }
    }

    /// takes a new strip from its first step, the caller decides whether the view draws again
    fn adopt_frames(&mut self, frames: Vec<Arc<RenderImage>>, delay: Duration, cx: &mut Context<Self>) {
        for previous in std::mem::replace(&mut self.frames, frames) {
            LottieRasterCache::release_image(previous, cx);
        }
        self.delay = delay;
        self.shown_frame.set(None);
        self.ticking = false;
        self._ticker = Task::ready(());
        self.started_at = RiseClock::monotonic();
    }

    /// a play once run keeps only the step it rests on
    /// if that step is already on screen nothing is drawn again
    fn finish_run(&mut self, cx: &mut Context<Self>) {
        self.finished = true;
        let Some(last) = self.frames.last().cloned() else {
            return;
        };
        let last_index = self.frames.len() - 1;
        let rests_on_screen = self.shown_frame.get() == Some(last_index);
        for (index, previous) in std::mem::take(&mut self.frames).into_iter().enumerate() {
            if index != last_index {
                LottieRasterCache::release_image(previous, cx);
            }
        }
        self.frames = vec![last];
        self.ticking = false;
        self._ticker = Task::ready(());
        if rests_on_screen {
            self.shown_frame.set(Some(0));
        } else {
            self.shown_frame.set(None);
            cx.notify();
        }
    }

    fn ensure_ticking(&mut self, window: AnyWindowHandle, key: u32, cx: &mut Context<Self>) {
        self.swap_window = Some(window);
        self.swap_key = key;
        if self.ticking || self.frames.len() < 2 || self.delay.is_zero() {
            return;
        }
        self.ticking = true;
        let delay = self.delay;
        self._ticker = cx.spawn(async move |player, cx| {
            loop {
                cx.background_executor().timer(delay).await;
                let keeps_ticking = player.update(cx, |player, cx| player.advance(cx)).unwrap_or(false);
                if !keeps_ticking {
                    break;
                }
            }
        });
    }

    fn advance(&mut self, cx: &mut Context<Self>) -> bool {
        let frame_count = self.frames.len();
        if !self.ticking || frame_count < 2 {
            self.ticking = false;
            return false;
        }
        let looping = self.configuration.is_some_and(|configuration| configuration.looping);
        let index = timeline_frame_index(
            RiseClock::monotonic().saturating_sub(self.started_at),
            self.delay,
            frame_count,
            looping,
        );
        let at_end = !looping && index + 1 >= frame_count;
        let shown = self.shown_frame.get();
        if shown == Some(index) {
            self.ticking = !at_end;
            return self.ticking;
        }
        let swapped = match (shown, self.swap_window) {
            (Some(shown), Some(window)) if shown < frame_count => {
                let previous = self.frames[shown].clone();
                let next = self.frames[index].clone();
                let key = self.swap_key;
                window
                    .update(cx, |_, window, _| window.swap_image(key, &previous, &next, 0))
                    .unwrap_or(false)
            }
            _ => false,
        };
        if swapped {
            self.shown_frame.set(Some(index));
            self.ticking = !at_end;
            return self.ticking;
        }
        self.ticking = false;
        cx.notify();
        false
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum RasterMode {
    Paused,
    Playing { speed: f32 },
}

/// one rasterisation, the animation plus pixel side plus speed (none while paused)
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
struct RasterKey {
    name: SharedString,
    dimension: u32,
    speed_bits: Option<u32>,
}

impl RasterKey {
    fn new(name: &SharedString, dimension: u32, mode: RasterMode) -> Self {
        let speed_bits = match mode {
            RasterMode::Paused => None,
            RasterMode::Playing { speed } => Some(speed.to_bits()),
        };
        Self { name: name.clone(), dimension, speed_bits }
    }
}

#[derive(Clone)]
struct CachedRaster {
    frames: Vec<Arc<RenderImage>>,
    delay: Duration,
    duration: Duration,
}

/// strips shared by every player of one animation at one size, eg a premium mark in every list row
/// a player never hands a shared picture back to the atlas, the cache does when it drops an entry
#[derive(Default)]
struct LottieRasterCache {
    entries: HashMap<RasterKey, CachedRaster>,
    recency: VecDeque<RasterKey>,
}

impl Global for LottieRasterCache {}

impl LottieRasterCache {
    const CAPACITY: usize = 24;

    fn get(key: &RasterKey, cx: &mut App) -> Option<CachedRaster> {
        let cache = cx.default_global::<Self>();
        let raster = cache.entries.get(key)?.clone();
        if let Some(position) = cache.recency.iter().position(|known| known == key) {
            cache.recency.remove(position);
        }
        cache.recency.push_back(key.clone());
        Some(raster)
    }

    fn insert(key: RasterKey, rendered: &Rendered, cx: &mut App) {
        let cache = cx.default_global::<Self>();
        if cache.entries.contains_key(&key) {
            return;
        }
        let raster =
            CachedRaster { frames: rendered.frames.clone(), delay: rendered.delay, duration: rendered.duration };
        cache.entries.insert(key.clone(), raster);
        cache.recency.push_back(key);
        let mut evicted = Vec::new();
        while cache.recency.len() > Self::CAPACITY {
            if let Some(oldest) = cache.recency.pop_front()
                && let Some(raster) = cache.entries.remove(&oldest)
            {
                evicted.extend(raster.frames);
            }
        }
        for image in evicted {
            if Arc::strong_count(&image) == 1 {
                cx.drop_image(image, None);
            }
        }
    }

    fn holds(image: &Arc<RenderImage>, cx: &mut App) -> bool {
        cx.default_global::<Self>()
            .entries
            .values()
            .any(|raster| raster.frames.iter().any(|cached| Arc::ptr_eq(cached, image)))
    }

    /// a player let a picture go, back to the atlas unless the cache keeps it for the next player
    fn release_image(image: Arc<RenderImage>, cx: &mut App) {
        if !Self::holds(&image, cx) {
            cx.drop_image(image, None);
        }
    }
}

struct Rendered {
    frames: Vec<Arc<RenderImage>>,
    delay: Duration,
    duration: Duration,
}

fn rasterize(model: &LottieViewModel, dimension: u32, mode: RasterMode) -> Option<Rendered> {
    let rasterizer = model.open_rasterizer()?;
    let key = SequenceKey::new(model.name.to_string(), dimension);
    let cache = Arc::new(DecodedFrameCache::new(raster_policy::MAXIMUM_BYTES_PER_SEQUENCE));
    let sequence = FrameSequence::open(key, rasterizer, cache).ok()?;

    let rendered = match mode {
        RasterMode::Paused => {
            let index = paused_frame_index(sequence.frame_count());
            let frame = sequence.frame(index)?;
            let delay = Delay::from_saturating_duration(Duration::from_secs(1));
            Rendered {
                frames: vec![single_frame_image(&frame, delay)?],
                delay: Duration::from(delay),
                duration: Duration::ZERO,
            }
        }
        RasterMode::Playing { speed } => {
            let stride = sequence.presentation_stride().max(1);
            let delay = frame_delay(sequence.frame_rate(), stride, speed);
            let mut indices: Vec<usize> = (0..sequence.frame_count()).step_by(stride).collect();
            if speed < 0.0 {
                indices.reverse();
            }
            let mut frames = Vec::with_capacity(indices.len());
            for index in &indices {
                let decoded = sequence.frame(*index)?;
                frames.push(single_frame_image(&decoded, delay)?);
            }
            if frames.is_empty() {
                return None;
            }
            let duration = Duration::from(delay) * u32::try_from(frames.len()).unwrap_or(u32::MAX);
            Rendered {
                frames,
                delay: Duration::from(delay),
                duration,
            }
        }
    };
    sequence.teardown();
    Some(rendered)
}

/// frame at the paused progress of a timeline of frame_count frames
fn paused_frame_index(frame_count: usize) -> usize {
    let last = frame_count.saturating_sub(1);
    ((last as f64) * PAUSED_PROGRESS).round() as usize
}

/// frame an evenly delayed timeline shows elapsed after it started
/// play once rests on the last frame and a loop wraps
fn timeline_frame_index(elapsed: Duration, delay: Duration, frame_count: usize, looping: bool) -> usize {
    let last = frame_count.saturating_sub(1);
    if last == 0 || delay.is_zero() {
        return 0;
    }
    let steps = elapsed.as_nanos() / delay.as_nanos();
    if looping {
        usize::try_from(steps % frame_count as u128).unwrap_or(0)
    } else {
        usize::try_from(steps).map_or(last, |steps| steps.min(last))
    }
}

fn frame_delay(source_frame_rate: f64, stride: usize, speed: f32) -> Delay {
    let rate = source_frame_rate.max(1.0) * f64::from(speed.abs()).max(f64::EPSILON);
    let seconds = stride as f64 / rate;
    Delay::from_saturating_duration(Duration::from_secs_f64(seconds.clamp(0.001, 1.0)))
}

fn frame_buffer(decoded: &DecodedFrame) -> Option<RgbaImage> {
    RgbaImage::from_raw(decoded.dimension, decoded.dimension, decoded.bytes().to_vec())
}

fn single_frame_image(decoded: &DecodedFrame, delay: Delay) -> Option<Arc<RenderImage>> {
    let frame = Frame::from_parts(frame_buffer(decoded)?, 0, 0, delay);
    let frames: SmallVec<[Frame; 1]> = SmallVec::from_buf([frame]);
    Some(Arc::new(RenderImage::new(frames)))
}

#[cfg(test)]
#[path = "lottie_view_tests.rs"]
mod tests;
