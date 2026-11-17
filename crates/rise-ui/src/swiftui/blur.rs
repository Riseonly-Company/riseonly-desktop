use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use gpui::{
    AnyElement, AnyImageCache, App, Bounds, Element, ElementId, Global, GlobalElementId,
    ImageCacheError, ImageId, ImageSource, ImgResourceLoader, InspectorElementId, IntoElement,
    LayoutId, Pixels, RenderImage, Size, Window,
};
use image::{Frame, RgbaImage};
use smallvec::SmallVec;

use crate::image_ui::rise_image_cache::RiseImageCache;

pub const BLURRED_IMAGE_CACHE_LIMIT: usize = 48;

const MIN_SOURCE_SIGMA: f32 = 0.25;

const SIGMA_STEPS_PER_PIXEL: f32 = 4.0;

const DOWNSCALE_SIGMA: f32 = 3.0;

/// one blur the content sits inside: radius in points, opaque flag and the blurred box
/// box is unknown during layout, known from prepaint
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BlurPass {
    pub radius: f32,
    pub opaque: bool,
    pub box_size: Option<Size<Pixels>>,
}

#[derive(Default)]
struct BlurStack(Vec<BlurPass>);

impl Global for BlurStack {}

/// blur the content being built or painted sits in, if any
pub fn current_blur(cx: &App) -> Option<BlurPass> {
    cx.try_global::<BlurStack>()
        .and_then(|stack| stack.0.last().copied())
}

/// blur radius for a component render, 0 if none
pub fn blur_radius(cx: &App) -> f32 {
    current_blur(cx).map_or(0.0, |pass| pass.radius)
}

/// two blurs in a row make one gaussian with the summed variance
pub fn combined_blur_radius(outer: f32, inner: f32) -> f32 {
    (outer * outer + inner * inner).sqrt()
}

fn push_blur(radius: f32, opaque: bool, box_size: Option<Size<Pixels>>, cx: &mut App) {
    let stack = &mut cx.default_global::<BlurStack>().0;
    let radius = stack
        .last()
        .map_or(radius, |outer| combined_blur_radius(outer.radius, radius));
    let opaque = opaque || stack.last().is_some_and(|outer| outer.opaque);
    stack.push(BlurPass {
        radius,
        opaque,
        box_size,
    });
}

fn pop_blur(cx: &mut App) {
    cx.default_global::<BlurStack>().0.pop();
}

pub trait BlurModifiers: IntoElement + Sized {
    fn blur(self, radius: f32) -> Blurred {
        Blurred::new(radius, false, self)
    }

    /// blurs the content by radius, opaque says if edges stay opaque
    fn blur_opaque(self, radius: f32, opaque: bool) -> Blurred {
        Blurred::new(radius, opaque, self)
    }
}

impl<E: IntoElement> BlurModifiers for E {}

/// element from blur, no layout node of its own
pub struct Blurred {
    radius: f32,
    opaque: bool,
    content: AnyElement,
}

impl Blurred {
    pub fn new(radius: f32, opaque: bool, content: impl IntoElement) -> Self {
        Self {
            radius: radius.max(0.0),
            opaque,
            content: content.into_any_element(),
        }
    }

    fn is_active(&self) -> bool {
        self.radius > 0.0
    }
}

impl IntoElement for Blurred {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for Blurred {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        if !self.is_active() {
            return (self.content.request_layout(window, cx), ());
        }
        push_blur(self.radius, self.opaque, None, cx);
        let layout_id = self.content.request_layout(window, cx);
        pop_blur(cx);
        (layout_id, ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        if !self.is_active() {
            self.content.prepaint(window, cx);
            return;
        }
        push_blur(self.radius, self.opaque, Some(bounds.size), cx);
        self.content.prepaint(window, cx);
        pop_blur(cx);
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        if !self.is_active() {
            self.content.paint(window, cx);
            return;
        }
        push_blur(self.radius, self.opaque, Some(bounds.size), cx);
        self.content.paint(window, cx);
        pop_blur(cx);
    }
}

/// wraps an image source so it follows the blur it is painted in
/// no blur or still in layout gives the source itself, while the blurred copy is made nothing is drawn
pub fn blur_aware_image_source(source: ImageSource) -> ImageSource {
    ImageSource::Custom(Arc::new(move |window: &mut Window, cx: &mut App| {
        let original = match load_source(&source, window, cx)? {
            Ok(original) => original,
            Err(error) => return Some(Err(error)),
        };
        let Some(pass) = current_blur(cx) else {
            return Some(Ok(original));
        };
        let Some(box_size) = pass.box_size else {
            return Some(Ok(original));
        };
        let pixels = original.size(0);
        let sigma = source_sigma(
            pass.radius,
            (pixels.width.0 as f32, pixels.height.0 as f32),
            (f32::from(box_size.width), f32::from(box_size.height)),
        );
        if sigma < MIN_SOURCE_SIGMA {
            return Some(Ok(original));
        }
        fetch_blurred(original, sigma, pass.opaque, window, cx).map(Ok)
    }))
}

fn load_source(
    source: &ImageSource,
    window: &mut Window,
    cx: &mut App,
) -> Option<Result<Arc<RenderImage>, ImageCacheError>> {
    match source {
        ImageSource::Resource(resource) => match RiseImageCache::shared(cx) {
            Some(cache) => AnyImageCache::from(cache).load(resource, window, cx),
            None => window.use_asset::<ImgResourceLoader>(resource, cx),
        },
        ImageSource::Render(image) => Some(Ok(image.clone())),
        ImageSource::Image(image) => image.clone().use_render_image(window, cx).map(Ok),
        ImageSource::Custom(load) => load(window, cx),
    }
}

/// gaussian sigma in source pixels for a blur radius over an image in a box, rounded to cache steps
pub fn source_sigma(radius: f32, image_pixels: (f32, f32), box_points: (f32, f32)) -> f32 {
    if radius <= 0.0 || image_pixels.0 <= 0.0 || image_pixels.1 <= 0.0 {
        return 0.0;
    }
    let points_per_pixel = (box_points.0 / image_pixels.0).max(box_points.1 / image_pixels.1);
    if points_per_pixel <= 0.0 || !points_per_pixel.is_finite() {
        return 0.0;
    }
    let sigma = radius / 2.0 / points_per_pixel;
    (sigma * SIGMA_STEPS_PER_PIXEL).round() / SIGMA_STEPS_PER_PIXEL
}

type BlurKey = (ImageId, u32, bool);

enum BlurredEntry {
    Loading,
    Ready { image: Arc<RenderImage>, unread: bool },
}

#[derive(Default)]
struct BlurredImageCache {
    entries: HashMap<BlurKey, BlurredEntry>,
    recency: VecDeque<BlurKey>,
}

impl Global for BlurredImageCache {}

impl BlurredImageCache {
    fn touch(&mut self, key: BlurKey) {
        if let Some(position) = self.recency.iter().position(|entry| *entry == key) {
            self.recency.remove(position);
        }
        self.recency.push_back(key);
    }

    fn evict_over_limit(&mut self) -> Vec<Arc<RenderImage>> {
        let mut freed = Vec::new();
        self.evict_where(BLURRED_IMAGE_CACHE_LIMIT, false, &mut freed);
        self.evict_where(BLURRED_IMAGE_CACHE_LIMIT * 2, true, &mut freed);
        freed
    }

    fn evict_where(&mut self, limit: usize, include_unread: bool, freed: &mut Vec<Arc<RenderImage>>) {
        let mut excess = self.recency.len().saturating_sub(limit);
        if excess == 0 {
            return;
        }
        let entries = &mut self.entries;
        self.recency.retain(|key| {
            if excess == 0 {
                return true;
            }
            let evictable = match entries.get(key) {
                Some(BlurredEntry::Ready { unread, .. }) => include_unread || !*unread,
                Some(BlurredEntry::Loading) => false,
                None => true,
            };
            if !evictable {
                return true;
            }
            if let Some(BlurredEntry::Ready { image, .. }) = entries.remove(key) {
                freed.push(image);
            }
            excess -= 1;
            false
        });
    }
}

fn fetch_blurred(
    original: Arc<RenderImage>,
    sigma: f32,
    opaque: bool,
    window: &mut Window,
    cx: &mut App,
) -> Option<Arc<RenderImage>> {
    let key: BlurKey = (
        original.id,
        (sigma * SIGMA_STEPS_PER_PIXEL).round() as u32,
        opaque,
    );
    let cache = cx.default_global::<BlurredImageCache>();
    if let Some(entry) = cache.entries.get_mut(&key) {
        let hit = match entry {
            BlurredEntry::Ready { image, unread } => {
                *unread = false;
                Some(image.clone())
            }
            BlurredEntry::Loading => None,
        };
        cache.touch(key);
        return hit;
    }
    cache.entries.insert(key, BlurredEntry::Loading);
    cache.recency.push_back(key);

    let task = cx
        .background_executor()
        .spawn(async move { gaussian_blur_render_image(&original, sigma, opaque) });
    let view = window.current_view();
    window
        .spawn(cx, async move |cx| {
            let blurred = Arc::new(task.await);
            let _ = cx.update(move |window, cx| {
                let cache = cx.default_global::<BlurredImageCache>();
                let Some(entry) = cache.entries.get_mut(&key) else {
                    return;
                };
                *entry = BlurredEntry::Ready { image: blurred, unread: true };
                let freed = cache.evict_over_limit();
                for image in freed {
                    cx.drop_image(image, Some(&mut *window));
                }
                cx.notify(view);
            });
        })
        .detach();
    None
}

/// blurred copy of every frame of the image, delays kept
pub fn gaussian_blur_render_image(image: &RenderImage, sigma: f32, opaque: bool) -> RenderImage {
    let frames: SmallVec<[Frame; 1]> = (0..image.frame_count())
        .filter_map(|index| {
            let size = image.size(index);
            let width = usize::try_from(size.width.0).ok()?;
            let height = usize::try_from(size.height.0).ok()?;
            let bytes = image.as_bytes(index)?;
            let (pixels, width, height) = blur_pixels(bytes, width, height, sigma, opaque)?;
            let buffer = RgbaImage::from_raw(
                u32::try_from(width).ok()?,
                u32::try_from(height).ok()?,
                pixels,
            )?;
            Some(Frame::from_parts(buffer, 0, 0, image.delay(index)))
        })
        .collect();
    RenderImage::new(frames)
}

/// blurs 4 channel 8 bit pixels with alpha last, other channel order doesnt matter
/// returns pixels and size, smaller than input for a wide blur
pub fn blur_pixels(
    bytes: &[u8],
    width: usize,
    height: usize,
    sigma: f32,
    opaque: bool,
) -> Option<(Vec<u8>, usize, usize)> {
    if width == 0 || height == 0 || bytes.len() < width * height * 4 {
        return None;
    }
    let premultiplied = premultiply(bytes, width, height);
    let factor = downscale_factor(sigma);
    let (mut plane, width, height) = if factor > 1 {
        box_downscale(&premultiplied, width, height, factor)
    } else {
        (premultiplied, width, height)
    };
    let kernel = gaussian_kernel(sigma / factor as f32);
    let mut scratch = vec![[0.0_f32; 4]; width * height];
    convolve(&plane, &mut scratch, width, height, &kernel, opaque, Axis::Horizontal);
    convolve(&scratch, &mut plane, width, height, &kernel, opaque, Axis::Vertical);
    Some((unpremultiply(&plane), width, height))
}

/// source pixels per side that one blurred pixel averages before the gaussian runs
pub fn downscale_factor(sigma: f32) -> usize {
    if sigma <= DOWNSCALE_SIGMA {
        return 1;
    }
    ((sigma / DOWNSCALE_SIGMA).floor() as usize).max(1)
}

/// normalised gaussian weights from -3 sigma to 3 sigma
pub fn gaussian_kernel(sigma: f32) -> Vec<f32> {
    let sigma = sigma.max(f32::EPSILON);
    let reach = (sigma * 3.0).ceil().max(1.0) as usize;
    let denominator = 2.0 * sigma * sigma;
    let mut weights: Vec<f32> = (0..=2 * reach)
        .map(|index| {
            let offset = index as f32 - reach as f32;
            (-offset * offset / denominator).exp()
        })
        .collect();
    let sum: f32 = weights.iter().sum();
    for weight in &mut weights {
        *weight /= sum;
    }
    weights
}

#[derive(Clone, Copy)]
enum Axis {
    Horizontal,
    Vertical,
}

fn convolve(
    source: &[[f32; 4]],
    target: &mut [[f32; 4]],
    width: usize,
    height: usize,
    kernel: &[f32],
    opaque: bool,
    axis: Axis,
) {
    let reach = (kernel.len() / 2) as isize;
    let (length, lines) = match axis {
        Axis::Horizontal => (width, height),
        Axis::Vertical => (height, width),
    };
    let index_of = |line: usize, position: usize| match axis {
        Axis::Horizontal => line * width + position,
        Axis::Vertical => position * width + line,
    };
    for line in 0..lines {
        for position in 0..length {
            let mut sum = [0.0_f32; 4];
            for (tap, weight) in kernel.iter().enumerate() {
                let sample = position as isize + tap as isize - reach;
                let sample = if (0..length as isize).contains(&sample) {
                    sample as usize
                } else if opaque {
                    sample.clamp(0, length as isize - 1) as usize
                } else {
                    continue;
                };
                for (total, value) in sum.iter_mut().zip(source[index_of(line, sample)]) {
                    *total += value * weight;
                }
            }
            target[index_of(line, position)] = sum;
        }
    }
}

fn premultiply(bytes: &[u8], width: usize, height: usize) -> Vec<[f32; 4]> {
    bytes
        .chunks_exact(4)
        .take(width * height)
        .map(|pixel| {
            let alpha = f32::from(pixel[3]) / 255.0;
            [
                f32::from(pixel[0]) * alpha,
                f32::from(pixel[1]) * alpha,
                f32::from(pixel[2]) * alpha,
                f32::from(pixel[3]),
            ]
        })
        .collect()
}

fn unpremultiply(plane: &[[f32; 4]]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(plane.len() * 4);
    for pixel in plane {
        let alpha = pixel[3].clamp(0.0, 255.0);
        let scale = if alpha > 0.0 { 255.0 / alpha } else { 0.0 };
        for channel in &pixel[..3] {
            bytes.push((channel * scale).round().clamp(0.0, 255.0) as u8);
        }
        bytes.push(alpha.round() as u8);
    }
    bytes
}

fn box_downscale(
    plane: &[[f32; 4]],
    width: usize,
    height: usize,
    factor: usize,
) -> (Vec<[f32; 4]>, usize, usize) {
    let small_width = width.div_ceil(factor);
    let small_height = height.div_ceil(factor);
    let mut small = vec![[0.0_f32; 4]; small_width * small_height];
    for small_y in 0..small_height {
        for small_x in 0..small_width {
            let mut sum = [0.0_f32; 4];
            let mut count = 0.0_f32;
            for y in small_y * factor..((small_y + 1) * factor).min(height) {
                for x in small_x * factor..((small_x + 1) * factor).min(width) {
                    for (total, value) in sum.iter_mut().zip(plane[y * width + x]) {
                        *total += value;
                    }
                    count += 1.0;
                }
            }
            if count > 0.0 {
                for channel in &mut sum {
                    *channel /= count;
                }
            }
            small[small_y * small_width + small_x] = sum;
        }
    }
    (small, small_width, small_height)
}
