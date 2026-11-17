use std::cell::Cell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::f32::consts::PI;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use gpui::{
    App, Bounds, ContentMask, Context, Corners, ElementId, Global, Hsla, IntoElement, ParentElement, Pixels, Point,
    Render, RenderImage, RenderOnce, Size, Styled, Task, Window, canvas, div, fill, point, px, size,
};
use image::imageops::{FilterType, resize};
use image::{Frame, RgbaImage};
use rise_core::foundation::foundation_date::RiseClock;
use smallvec::smallvec;

use crate::swiftui::blur::blur_pixels;
use crate::swiftui::environment::reduce_motion;
use crate::swiftui::motion::CubicBezier;

pub type MediaSpoilerRevealCompletion = Box<dyn FnOnce(&mut App)>;

/// emitter of one tile, when it started and the size it covers
#[derive(Clone, Copy, PartialEq)]
struct MediaSpoilerEmitter {
    begin_time: Duration,
    size: Size<Pixels>,
    births_stopped_at: Option<Duration>,
}

/// the circle growing from the finger while the spoiler opens
#[derive(Clone, Copy)]
struct MediaSpoilerRevealMask {
    origin: Point<Pixels>,
    far: f32,
    began: Duration,
}

/// photo or video hidden under a spoiler, like Telegram
/// blurred copy under twinkling dust, same as a text spoiler but thinner
/// a tap opens it with a circle growing from the finger
pub struct MediaSpoilerOverlayView {
    blur_image: Option<Arc<RenderImage>>,
    blur_key: Option<String>,
    emitter: Rc<Cell<Option<MediaSpoilerEmitter>>>,
    bounds_size: Rc<Cell<Size<Pixels>>>,
    is_revealing: bool,
    is_hidden: bool,
    mask: Option<MediaSpoilerRevealMask>,
    reveal_completion: Option<MediaSpoilerRevealCompletion>,
    reveal_task: Option<Task<()>>,
}

impl Default for MediaSpoilerOverlayView {
    fn default() -> Self {
        Self::new()
    }
}

impl MediaSpoilerOverlayView {
    const LIFETIME: f32 = 1.6;
    const LIFETIME_RANGE: f32 = 0.7;
    const DENSITY: f32 = 0.011;
    const WAVE_DURATION: f32 = 2.6;
    const MAX_BIRTH_RATE: f32 = 2400.0;
    const REVEAL_DURATION: f32 = 0.45;
    const VELOCITY: f32 = 6.0;
    const VELOCITY_RANGE: f32 = 8.0;
    const SCALE_RANGE: f32 = 0.5;
    const ALPHA_RANGE: f32 = 0.6;
    const ALPHA_SPEED: f32 = -0.45;
    const DOT_SIDE: f32 = 1.6;
    const MASK_BAND: f32 = 2.0;

    pub fn new() -> Self {
        Self {
            blur_image: None,
            blur_key: None,
            emitter: Rc::new(Cell::new(None)),
            bounds_size: Rc::new(Cell::new(Size::default())),
            is_revealing: false,
            is_hidden: false,
            mask: None,
            reveal_completion: None,
            reveal_task: None,
        }
    }

    pub fn is_revealing(&self) -> bool {
        self.is_revealing
    }

    pub fn is_hidden(&self) -> bool {
        self.is_hidden
    }

    /// picture under the spoiler, key names the source so a reused tile doesnt blur the same bitmap twice
    pub fn set_source(&mut self, image: Option<Arc<RenderImage>>, key: &str, cx: &mut Context<Self>) {
        let Some(image) = image else {
            if self.blur_key.is_some() {
                self.blur_key = None;
                self.blur_image = None;
                cx.notify();
            }
            return;
        };
        if self.blur_key.as_deref() == Some(key) && self.blur_image.is_some() {
            return;
        }
        self.blur_key = Some(key.to_string());
        self.blur_image = MediaSpoilerBlur::blurred(&image, key, cx);
        cx.notify();
    }

    /// closes the spoiler again without animation (a reused tile or new message)
    pub fn reset(&mut self, cx: &mut Context<Self>) {
        self.is_revealing = false;
        self.reveal_task = None;
        self.reveal_completion = None;
        self.mask = None;
        self.is_hidden = false;
        self.emitter.set(None);
        cx.notify();
    }

    /// opens the spoiler from point (in this view) or from the center
    pub fn reveal(
        &mut self,
        from: Option<Point<Pixels>>,
        animated: bool,
        completion: Option<MediaSpoilerRevealCompletion>,
        cx: &mut Context<Self>,
    ) {
        if self.is_hidden || self.is_revealing {
            if let Some(completion) = completion {
                completion(cx);
            }
            return;
        }
        let bounds = self.bounds_size.get();
        let width = f32::from(bounds.width);
        let height = f32::from(bounds.height);
        if !animated || width <= 1.0 || height <= 1.0 || reduce_motion(cx) {
            self.is_hidden = true;
            cx.notify();
            if let Some(completion) = completion {
                completion(cx);
            }
            return;
        }
        self.is_revealing = true;
        let now = RiseClock::monotonic();
        if let Some(mut emitter) = self.emitter.get() {
            emitter.births_stopped_at = Some(now);
            self.emitter.set(Some(emitter));
        }
        let origin = from.unwrap_or_else(|| point(px(width / 2.0), px(height / 2.0)));
        let origin_x = f32::from(origin.x);
        let origin_y = f32::from(origin.y);
        let far = [(0.0, 0.0), (width, 0.0), (0.0, height), (width, height)]
            .iter()
            .map(|(x, y)| (x - origin_x).hypot(y - origin_y))
            .fold(0.0_f32, f32::max);
        self.mask = Some(MediaSpoilerRevealMask { origin, far: far + 2.0, began: now });
        self.reveal_completion = completion;
        let duration = Duration::from_secs_f32(Self::REVEAL_DURATION);
        self.reveal_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(duration).await;
            this.update(cx, |view, cx| view.finish_reveal(cx)).ok();
        }));
        cx.notify();
    }

    fn finish_reveal(&mut self, cx: &mut Context<Self>) {
        if !self.is_revealing {
            return;
        }
        self.is_revealing = false;
        self.is_hidden = true;
        self.mask = None;
        self.reveal_task = None;
        cx.notify();
        if let Some(completion) = self.reveal_completion.take() {
            completion(cx);
        }
    }

    fn layout_dust(emitter: &Cell<Option<MediaSpoilerEmitter>>, tile: Size<Pixels>, is_revealing: bool, now: Duration) {
        if is_revealing || f32::from(tile.width) <= 1.0 || f32::from(tile.height) <= 1.0 {
            return;
        }
        if emitter.get().is_some_and(|current| current.size == tile) {
            return;
        }
        let begin_time = now.saturating_sub(Duration::from_secs_f32(Self::LIFETIME));
        emitter.set(Some(MediaSpoilerEmitter { begin_time, size: tile, births_stopped_at: None }));
    }

    fn paint_layers(bounds: Bounds<Pixels>, blur_image: Option<&Arc<RenderImage>>, window: &mut Window) {
        window.paint_quad(fill(bounds, Hsla { h: 0.0, s: 0.0, l: 0.14, a: 1.0 }));
        if let Some(image) = blur_image {
            let image_size = image.size(0);
            let image_width = image_size.width.0 as f32;
            let image_height = image_size.height.0 as f32;
            if image_width > 0.0 && image_height > 0.0 {
                let width = f32::from(bounds.size.width);
                let height = f32::from(bounds.size.height);
                let scale = f32::max(width / image_width, height / image_height);
                let drawn = size(px(image_width * scale), px(image_height * scale));
                let image_bounds = Bounds::new(
                    point(
                        bounds.origin.x + (bounds.size.width - drawn.width) / 2.0,
                        bounds.origin.y + (bounds.size.height - drawn.height) / 2.0,
                    ),
                    drawn,
                );
                window.paint_image(bounds, image_bounds, Corners::default(), image.clone(), 0, false).ok();
            }
        }
        window.paint_quad(fill(bounds, Hsla { h: 0.0, s: 0.0, l: 0.0, a: 0.2 }));
    }

    fn paint_masked(
        bounds: Bounds<Pixels>,
        blur_image: Option<&Arc<RenderImage>>,
        hole: Option<(Point<Pixels>, f32)>,
        window: &mut Window,
    ) {
        let Some((center, radius)) = hole else {
            Self::paint_layers(bounds, blur_image, window);
            return;
        };
        let top = f32::from(bounds.origin.y);
        let bottom = top + f32::from(bounds.size.height);
        let left = f32::from(bounds.origin.x);
        let right = left + f32::from(bounds.size.width);
        let center_x = left + f32::from(center.x);
        let center_y = top + f32::from(center.y);
        let mut regions: Vec<Bounds<Pixels>> = Vec::new();
        let hole_top = (center_y - radius).max(top);
        let hole_bottom = (center_y + radius).min(bottom);
        if hole_top > top {
            regions.push(Self::region(left, top, right, hole_top));
        }
        let mut band = hole_top;
        while band < hole_bottom {
            let band_end = (band + Self::MASK_BAND).min(hole_bottom);
            let nearest = center_y.clamp(band, band_end);
            let half = (radius * radius - (nearest - center_y).powi(2)).max(0.0).sqrt();
            let gap_left = (center_x - half).max(left);
            let gap_right = (center_x + half).min(right);
            if gap_left > left {
                regions.push(Self::region(left, band, gap_left, band_end));
            }
            if gap_right < right {
                regions.push(Self::region(gap_right.max(left), band, right, band_end));
            }
            band = band_end;
        }
        if hole_bottom < bottom {
            regions.push(Self::region(left, hole_bottom.max(top), right, bottom));
        }
        for region in regions {
            window.with_content_mask(Some(ContentMask { bounds: region }), |window| {
                Self::paint_layers(bounds, blur_image, window);
            });
        }
    }

    fn region(left: f32, top: f32, right: f32, bottom: f32) -> Bounds<Pixels> {
        Bounds::new(point(px(left), px(top)), size(px((right - left).max(0.0)), px((bottom - top).max(0.0))))
    }

    fn wave_mask(along: f32, now: f32) -> f32 {
        let pass = (now / Self::WAVE_DURATION).fract();
        let eased = CubicBezier(0.42, 0.0, 0.58, 1.0).progress(f64::from(pass)) as f32;
        let shift = 1.5 * eased;
        let locations = [-0.5 + shift, -0.25 + shift, shift];
        let dim = 0.3;
        if along <= locations[0] || along >= locations[2] {
            return dim;
        }
        if along <= locations[1] {
            let fraction = (along - locations[0]) / (locations[1] - locations[0]);
            return dim + (1.0 - dim) * fraction;
        }
        let fraction = (along - locations[1]) / (locations[2] - locations[1]);
        1.0 + (dim - 1.0) * fraction
    }

    fn paint_dust(
        bounds: Bounds<Pixels>,
        emitter: MediaSpoilerEmitter,
        hole: Option<(Point<Pixels>, f32)>,
        now: Duration,
        window: &mut Window,
    ) {
        let width = f32::from(bounds.size.width);
        let height = f32::from(bounds.size.height);
        let alive = width * height * Self::DENSITY;
        let birth_rate = Self::MAX_BIRTH_RATE.min(alive / Self::LIFETIME);
        let slots = (birth_rate * Self::LIFETIME).ceil().max(1.0) as u64;
        let elapsed = now.saturating_sub(emitter.begin_time).as_secs_f32();
        let stop = emitter
            .births_stopped_at
            .map(|stopped| stopped.saturating_sub(emitter.begin_time).as_secs_f32());
        let wave_clock = now.as_secs_f32();
        for slot in 0..slots {
            let lifetime = emitter_spread(Self::LIFETIME, Self::LIFETIME_RANGE, particle_unit(slot, 0, 0)).max(0.05);
            let clock = elapsed + particle_unit(slot, 0, 1) * lifetime;
            let generation = (clock / lifetime).floor();
            let age = clock - generation * lifetime;
            if let Some(stop) = stop
                && elapsed - age > stop
            {
                continue;
            }
            let generation = generation as u64;
            let base_alpha = emitter_spread(1.0, Self::ALPHA_RANGE, particle_unit(slot, generation, 2));
            let alpha = (base_alpha + Self::ALPHA_SPEED * age).clamp(0.0, 1.0);
            if alpha <= 0.0 {
                continue;
            }
            let angle = particle_unit(slot, generation, 3) * 2.0 * PI;
            let speed = emitter_spread(Self::VELOCITY, Self::VELOCITY_RANGE, particle_unit(slot, generation, 4));
            let x = particle_unit(slot, generation, 5) * width + angle.cos() * speed * age;
            let y = particle_unit(slot, generation, 6) * height + angle.sin() * speed * age;
            if let Some((center, radius)) = hole
                && (x - f32::from(center.x)).hypot(y - f32::from(center.y)) < radius
            {
                continue;
            }
            let along = (x / width.max(1.0) + y / height.max(1.0)) / 2.0;
            let alpha = alpha * Self::wave_mask(along, wave_clock);
            let side = Self::DOT_SIDE * emitter_spread(1.0, Self::SCALE_RANGE, particle_unit(slot, generation, 7));
            let dot_bounds = Bounds::new(
                point(bounds.origin.x + px(x - side / 2.0), bounds.origin.y + px(y - side / 2.0)),
                size(px(side), px(side)),
            );
            window.paint_quad(fill(dot_bounds, Hsla { h: 0.0, s: 0.0, l: 1.0, a: alpha }).corner_radii(px(side / 2.0)));
        }
    }
}

impl Render for MediaSpoilerOverlayView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        if self.is_hidden {
            return div();
        }
        let blur_image = self.blur_image.clone();
        let emitter = self.emitter.clone();
        let bounds_size = self.bounds_size.clone();
        let is_revealing = self.is_revealing;
        let mask = self.mask;
        let layer = canvas(
            |_, _, _| {},
            move |bounds, _, window, _| {
                let now = RiseClock::monotonic();
                bounds_size.set(bounds.size);
                Self::layout_dust(&emitter, bounds.size, is_revealing, now);
                let hole = mask.map(|mask| {
                    let progress = (now.saturating_sub(mask.began).as_secs_f32() / Self::REVEAL_DURATION).clamp(0.0, 1.0);
                    let eased = CubicBezier(0.42, 0.0, 1.0, 1.0).progress(f64::from(progress)) as f32;
                    (mask.origin, mask.far * eased)
                });
                window.with_content_mask(Some(ContentMask { bounds }), |window| {
                    Self::paint_masked(bounds, blur_image.as_ref(), hole, window);
                    if let Some(current) = emitter.get() {
                        Self::paint_dust(bounds, current, hole, now, window);
                    }
                });
                window.request_animation_frame();
            },
        )
        .size_full();
        div().size_full().overflow_hidden().child(layer)
    }
}

/// emitter value with its range, uniform in value minus range to value plus range
fn emitter_spread(value: f32, range: f32, unit: f32) -> f32 {
    value + (2.0 * unit - 1.0) * range
}

/// uniform number in 0 to 1 for one particle draw, stable across frames
fn particle_unit(slot: u64, generation: u64, channel: u64) -> f32 {
    let mut state = slot.wrapping_mul(0xBF58_476D_1CE4_E5B9)
        ^ generation.wrapping_mul(0x94D0_49BB_1331_11EB)
        ^ channel.wrapping_mul(0xD6E8_FEB8_6659_FD93)
        ^ 0x9E37_79B9_7F4A_7C15;
    state ^= state >> 30;
    state = state.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    state ^= state >> 27;
    state = state.wrapping_mul(0x94D0_49BB_1331_11EB);
    state ^= state >> 31;
    (state >> 40) as f32 / (1u64 << 24) as f32
}

/// host of the same spoiler for previews that are not tiles
#[derive(IntoElement)]
pub struct MediaSpoilerOverlay {
    image: Option<Arc<RenderImage>>,
    key: String,
}

impl MediaSpoilerOverlay {
    pub fn new(image: Option<Arc<RenderImage>>, key: impl Into<String>) -> Self {
        Self { image, key: key.into() }
    }
}

impl RenderOnce for MediaSpoilerOverlay {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let id = ElementId::Name(format!("media-spoiler-{}", self.key).into());
        let view = window.use_keyed_state(id, cx, |_, _| MediaSpoilerOverlayView::new());
        let key = self.key;
        let image = self.image;
        view.update(cx, |view, cx| view.set_source(image, &key, cx));
        div().size_full().child(view)
    }
}

#[derive(Default)]
struct MediaSpoilerBlurCache {
    entries: HashMap<String, Arc<RenderImage>>,
    recency: VecDeque<String>,
}

impl Global for MediaSpoilerBlurCache {}

pub enum MediaSpoilerBlur {}

impl MediaSpoilerBlur {
    const COUNT_LIMIT: usize = 120;
    const SIDE: f32 = 72.0;
    const SIGMA: f32 = 4.0;

    pub fn blurred(image: &Arc<RenderImage>, key: &str, cx: &mut App) -> Option<Arc<RenderImage>> {
        if let Some(hit) = cx.try_global::<MediaSpoilerBlurCache>().and_then(|cache| cache.entries.get(key).cloned()) {
            return Some(hit);
        }
        if image.frame_count() == 0 {
            return None;
        }
        let source_size = image.size(0);
        let width = u32::try_from(source_size.width.0).ok()?;
        let height = u32::try_from(source_size.height.0).ok()?;
        if width == 0 || height == 0 {
            return None;
        }
        let bytes = image.as_bytes(0)?;
        let source = RgbaImage::from_raw(width, height, bytes.to_vec())?;
        let scale = f32::min(1.0, Self::SIDE / (width.max(height) as f32));
        let target_width = ((width as f32 * scale).round() as u32).max(1);
        let target_height = ((height as f32 * scale).round() as u32).max(1);
        let small = resize(&source, target_width, target_height, FilterType::Triangle);
        let Some((pixels, blurred_width, blurred_height)) = blur_pixels(
            small.as_raw(),
            target_width as usize,
            target_height as usize,
            Self::SIGMA,
            true,
        ) else {
            return Some(Arc::new(RenderImage::new(smallvec![Frame::new(small)])));
        };
        let Some(buffer) = RgbaImage::from_raw(
            u32::try_from(blurred_width).ok()?,
            u32::try_from(blurred_height).ok()?,
            pixels,
        ) else {
            return Some(Arc::new(RenderImage::new(smallvec![Frame::new(small)])));
        };
        let result = Arc::new(RenderImage::new(smallvec![Frame::new(buffer)]));
        let cache = cx.default_global::<MediaSpoilerBlurCache>();
        cache.entries.insert(key.to_string(), result.clone());
        cache.recency.retain(|entry| entry != key);
        cache.recency.push_back(key.to_string());
        while cache.recency.len() > Self::COUNT_LIMIT {
            let Some(oldest) = cache.recency.pop_front() else {
                break;
            };
            cache.entries.remove(&oldest);
        }
        Some(result)
    }
}

#[derive(Default)]
struct MediaSpoilerOpened(HashSet<String>);

impl Global for MediaSpoilerOpened {}

/// spoilers opened in this launch by tile identity
/// a tile scrolled away and back stays open
pub enum MediaSpoilerReveals {}

impl MediaSpoilerReveals {
    pub fn is_opened(id: &str, cx: &App) -> bool {
        cx.try_global::<MediaSpoilerOpened>().is_some_and(|opened| opened.0.contains(id))
    }

    pub fn open(id: &str, cx: &mut App) {
        cx.default_global::<MediaSpoilerOpened>().0.insert(id.to_string());
    }
}
