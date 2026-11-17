use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;

use gpui::{
    AnyElement, App, Bounds, BoxShadow, Corners, Element, ElementId, EntityId, Global, GlobalElementId, Hsla, ImageId,
    InspectorElementId, IntoElement, LayoutId, ObjectFit, Pixels, RenderImage, Rgba, Window, point, px, size,
};
use image::{Frame, RgbaImage};
use smallvec::SmallVec;

use super::blur::{gaussian_kernel, source_sigma};

pub const ALPHA_SHADOW_CACHE_LIMIT: usize = 256;

const ALPHA_SHADOW_IN_FLIGHT_LIMIT: usize = 4;

const WORKING_SIGMA: f32 = 1.5;

const ALPHA_SHADOW_MASK_SIDE: usize = 128;

const SIGMA_STEPS_PER_PIXEL: f32 = 4.0;

/// shadow settings, color and radius and x and y offset in points
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AlphaShadow {
    pub color: Hsla,
    pub radius: f32,
    pub x: f32,
    pub y: f32,
}

/// how a picture of a given sigma becomes a shadow bitmap
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AlphaShadowPlan {
    pub factor: usize,
    pub working_sigma: f32,
    pub padding: usize,
}

impl AlphaShadowPlan {
    pub fn new(sigma: f32) -> Self {
        let factor = ((sigma / WORKING_SIGMA).floor() as usize).max(1);
        let working_sigma = sigma / factor as f32;
        let padding = (working_sigma * 3.0).ceil().max(1.0) as usize;
        Self {
            factor,
            working_sigma,
            padding,
        }
    }

    /// size of the shadow bitmap for a picture of width by height pixels
    pub fn baked_size(&self, width: usize, height: usize) -> (usize, usize) {
        (
            width.div_ceil(self.factor) + 2 * self.padding,
            height.div_ceil(self.factor) + 2 * self.padding,
        )
    }
}

/// where the shadow bitmap lands for a picture drawn into the fitted box
pub fn alpha_shadow_bounds(
    fitted: Bounds<Pixels>,
    pixels: (usize, usize),
    plan: AlphaShadowPlan,
    offset: (f32, f32),
) -> Bounds<Pixels> {
    let (baked_width, baked_height) = plan.baked_size(pixels.0, pixels.1);
    let step_x = f32::from(fitted.size.width) / pixels.0 as f32 * plan.factor as f32;
    let step_y = f32::from(fitted.size.height) / pixels.1 as f32 * plan.factor as f32;
    Bounds {
        origin: point(
            px(f32::from(fitted.origin.x) - plan.padding as f32 * step_x + offset.0),
            px(f32::from(fitted.origin.y) - plan.padding as f32 * step_y + offset.1),
        ),
        size: size(px(baked_width as f32 * step_x), px(baked_height as f32 * step_y)),
    }
}

/// shadow of packed BGRA rows, alpha blurred and filled with the color
/// channels are not premultiplied, like a decoded gpui image
pub fn alpha_shadow_pixels(
    bgra: &[u8],
    width: usize,
    height: usize,
    plan: AlphaShadowPlan,
    color: [u8; 4],
) -> Option<(Vec<u8>, usize, usize)> {
    if width == 0 || height == 0 || bgra.len() < width * height * 4 {
        return None;
    }
    let (baked_width, baked_height) = plan.baked_size(width, height);
    let mut plane = vec![0.0_f32; baked_width * baked_height];
    for (row, pixels) in bgra.chunks_exact(width * 4).take(height).enumerate() {
        let line = (row / plan.factor + plan.padding) * baked_width;
        for (column, pixel) in pixels.chunks_exact(4).enumerate() {
            plane[line + column / plan.factor + plan.padding] += f32::from(pixel[3]);
        }
    }
    let scale = 1.0 / (255.0 * (plan.factor * plan.factor) as f32);
    for value in &mut plane {
        *value *= scale;
    }
    let kernel = gaussian_kernel(plan.working_sigma);
    let mut scratch = vec![0.0_f32; plane.len()];
    spread(&plane, &mut scratch, baked_width, baked_height, &kernel, 1, baked_width);
    plane.fill(0.0);
    spread(&scratch, &mut plane, baked_height, baked_width, &kernel, baked_width, 1);

    let alpha = f32::from(color[3]);
    let mut baked = Vec::with_capacity(plane.len() * 4);
    for value in plane {
        baked.extend_from_slice(&[color[0], color[1], color[2], (value * alpha).round().clamp(0.0, 255.0) as u8]);
    }
    Some((baked, baked_width, baked_height))
}

/// one pass of the separable gaussian, along steps inside a line and across steps to the next
fn spread(
    source: &[f32],
    target: &mut [f32],
    length: usize,
    lines: usize,
    kernel: &[f32],
    along: usize,
    across: usize,
) {
    let reach = kernel.len() / 2;
    for line in 0..lines {
        for position in 0..length {
            let value = source[line * across + position * along];
            if value == 0.0 {
                continue;
            }
            let first = position.saturating_sub(reach);
            let last = (position + reach).min(length - 1);
            for neighbour in first..=last {
                let weight = kernel[neighbour + reach - position];
                target[line * across + neighbour * along] += value * weight;
            }
        }
    }
}

/// reduced alpha of packed BGRA rows plus its size, one sample per block is enough
pub fn alpha_shadow_mask_pixels(
    bgra: &[u8],
    width: usize,
    height: usize,
    stride: usize,
) -> Option<(Vec<u8>, usize, usize)> {
    if width == 0 || height == 0 || stride < width * 4 || bgra.len() < stride * height {
        return None;
    }
    let factor = width.max(height).div_ceil(ALPHA_SHADOW_MASK_SIDE).max(1);
    let mask_width = width.div_ceil(factor);
    let mask_height = height.div_ceil(factor);
    let mut mask = vec![0_u8; mask_width * mask_height * 4];
    for (index, pixel) in mask.chunks_exact_mut(4).enumerate() {
        let row = (index / mask_width * factor + factor / 2).min(height - 1);
        let column = (index % mask_width * factor + factor / 2).min(width - 1);
        pixel[3] = bgra[row * stride + column * 4 + 3];
    }
    Some((mask, mask_width, mask_height))
}

/// picture that only carries alpha, for frames on a surface the main thread cant read
/// never painted so no atlas tile and no drop_image needed
pub fn alpha_shadow_mask(bgra: &[u8], width: usize, height: usize, stride: usize) -> Option<Arc<RenderImage>> {
    let (mask, mask_width, mask_height) = alpha_shadow_mask_pixels(bgra, width, height, stride)?;
    let image = RenderImage::new(single_frame(mask, mask_width, mask_height)?);
    Some(Arc::new(image))
}

fn single_frame(pixels: Vec<u8>, width: usize, height: usize) -> Option<SmallVec<[Frame; 1]>> {
    let buffer = RgbaImage::from_raw(u32::try_from(width).ok()?, u32::try_from(height).ok()?, pixels)?;
    let mut frames: SmallVec<[Frame; 1]> = SmallVec::new();
    frames.push(Frame::new(buffer));
    Some(frames)
}

fn bake_alpha_shadow(image: &RenderImage, plan: AlphaShadowPlan, color: [u8; 4]) -> Option<RenderImage> {
    let pixels = image.size(0);
    let width = usize::try_from(pixels.width.0).ok()?;
    let height = usize::try_from(pixels.height.0).ok()?;
    let bytes = image.as_bytes(0)?;
    let (baked, baked_width, baked_height) = alpha_shadow_pixels(bytes, width, height, plan, color)?;
    Some(RenderImage::new(single_frame(baked, baked_width, baked_height)?))
}

type AlphaShadowKey = (ImageId, u32, [u8; 4]);

enum AlphaShadowEntryState {
    Loading,
    Ready(Arc<RenderImage>),
    Empty,
}

struct AlphaShadowEntry {
    state: AlphaShadowEntryState,
    last_used: u64,
}

#[derive(Default)]
struct AlphaShadowCache {
    entries: HashMap<AlphaShadowKey, AlphaShadowEntry>,
    clock: u64,
    in_flight: usize,
}

impl Global for AlphaShadowCache {}

impl AlphaShadowCache {
    fn ready(&mut self, key: &AlphaShadowKey) -> Option<Arc<RenderImage>> {
        self.clock += 1;
        let clock = self.clock;
        let entry = self.entries.get_mut(key)?;
        entry.last_used = clock;
        match &entry.state {
            AlphaShadowEntryState::Ready(image) => Some(image.clone()),
            AlphaShadowEntryState::Loading | AlphaShadowEntryState::Empty => None,
        }
    }

    fn evict_over_limit(&mut self) -> Vec<Arc<RenderImage>> {
        let mut freed = Vec::new();
        while self.entries.len() > ALPHA_SHADOW_CACHE_LIMIT {
            let oldest = self
                .entries
                .iter()
                .filter(|(_, entry)| !matches!(entry.state, AlphaShadowEntryState::Loading))
                .min_by_key(|(_, entry)| entry.last_used)
                .map(|(key, _)| *key);
            let Some(entry) = oldest.and_then(|key| self.entries.remove(&key)) else {
                break;
            };
            if let AlphaShadowEntryState::Ready(image) = entry.state {
                freed.push(image);
            }
        }
        freed
    }
}

fn request_alpha_shadow(
    image: &Arc<RenderImage>,
    key: AlphaShadowKey,
    plan: AlphaShadowPlan,
    window: &Window,
    cx: &mut App,
) -> Option<Arc<RenderImage>> {
    let cache = cx.default_global::<AlphaShadowCache>();
    if cache.entries.contains_key(&key) {
        return cache.ready(&key);
    }
    if cache.in_flight >= ALPHA_SHADOW_IN_FLIGHT_LIMIT {
        return None;
    }
    cache.in_flight += 1;
    cache.clock += 1;
    let last_used = cache.clock;
    cache.entries.insert(
        key,
        AlphaShadowEntry {
            state: AlphaShadowEntryState::Loading,
            last_used,
        },
    );

    let source = image.clone();
    let color = key.2;
    let task = cx
        .background_executor()
        .spawn(async move { bake_alpha_shadow(&source, plan, color).map(Arc::new) });
    let view = window.current_view();
    cx.spawn(async move |cx| {
        let baked = task.await;
        cx.update(move |cx| finish_alpha_shadow(key, baked, view, cx));
    })
    .detach();
    None
}

fn finish_alpha_shadow(key: AlphaShadowKey, baked: Option<Arc<RenderImage>>, view: EntityId, cx: &mut App) {
    let cache = cx.default_global::<AlphaShadowCache>();
    cache.in_flight = cache.in_flight.saturating_sub(1);
    if let Some(entry) = cache.entries.get_mut(&key) {
        entry.state = match baked {
            Some(image) => AlphaShadowEntryState::Ready(image),
            None => AlphaShadowEntryState::Empty,
        };
    }
    let freed = cache.evict_over_limit();
    for image in freed {
        cx.drop_image(image, None);
    }
    cx.notify(view);
}

/// what a source inside the scope drew
pub enum ReportedFrame {
    Image {
        image: Arc<RenderImage>,
        fitted: Bounds<Pixels>,
    },
    Opaque {
        bounds: Bounds<Pixels>,
        corner_radius: f32,
    },
}

thread_local! {
    static SCOPES: RefCell<Vec<Option<ReportedFrame>>> = const { RefCell::new(Vec::new()) };
}

fn is_inside_scope() -> bool {
    SCOPES.with_borrow(|scopes| !scopes.is_empty())
}

/// top content wins, a poster under a playing frame must not double the shadow
fn report(frame: ReportedFrame) {
    SCOPES.with_borrow_mut(|scopes| {
        if let Some(top) = scopes.last_mut() {
            *top = Some(frame);
        }
    });
}

pub trait AlphaShadowModifiers: IntoElement + Sized {
    /// shadow of whatever the sources inside drew
    fn alpha_shadow(self, id: impl Into<ElementId>, shadow: AlphaShadow) -> AlphaShadowed {
        AlphaShadowed {
            id: id.into(),
            shadow,
            content: self.into_any_element(),
        }
    }

    /// marks the element as drawing this image with this fit
    /// multi frame images use the first frame
    fn alpha_shadow_source(self, image: Arc<RenderImage>, fit: ObjectFit) -> AlphaShadowSource {
        AlphaShadowSource {
            kind: AlphaShadowSourceKind::Image { image, fit },
            content: self.into_any_element(),
        }
    }

    fn alpha_shadow_opaque_source(self, corner_radius: f32) -> AlphaShadowSource {
        AlphaShadowSource {
            kind: AlphaShadowSourceKind::Opaque { corner_radius },
            content: self.into_any_element(),
        }
    }
}

impl<E: IntoElement> AlphaShadowModifiers for E {}

enum AlphaShadowSourceKind {
    Image { image: Arc<RenderImage>, fit: ObjectFit },
    Opaque { corner_radius: f32 },
}

/// element from alpha_shadow_source, no layout node of its own
pub struct AlphaShadowSource {
    kind: AlphaShadowSourceKind,
    content: AnyElement,
}

impl IntoElement for AlphaShadowSource {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for AlphaShadowSource {
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
        (self.content.request_layout(window, cx), ())
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
        if is_inside_scope() {
            match &self.kind {
                AlphaShadowSourceKind::Image { image, fit } => {
                    let pixels = image.size(0);
                    if pixels.width.0 > 0 && pixels.height.0 > 0 {
                        report(ReportedFrame::Image {
                            image: image.clone(),
                            fitted: fit.get_bounds(bounds, pixels),
                        });
                    }
                }
                AlphaShadowSourceKind::Opaque { corner_radius } => {
                    report(ReportedFrame::Opaque {
                        bounds,
                        corner_radius: *corner_radius,
                    });
                }
            }
        }
        self.content.prepaint(window, cx);
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.content.paint(window, cx);
    }
}

pub struct AlphaShadowed {
    id: ElementId,
    shadow: AlphaShadow,
    content: AnyElement,
}

impl IntoElement for AlphaShadowed {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

/// last shadow this scope painted, so animations have something to show while the next is made
/// its a key not the image, painting the image again would leak a tile
#[derive(Clone, Copy)]
struct ShownAlphaShadow {
    key: AlphaShadowKey,
    pixels: (usize, usize),
    sigma: f32,
}

impl Element for AlphaShadowed {
    type RequestLayoutState = ();
    type PrepaintState = Option<ReportedFrame>;

    fn id(&self) -> Option<ElementId> {
        Some(self.id.clone())
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
        (self.content.request_layout(window, cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        SCOPES.with_borrow_mut(|scopes| scopes.push(None));
        self.content.prepaint(window, cx);
        SCOPES.with_borrow_mut(|scopes| scopes.pop().flatten())
    }

    fn paint(
        &mut self,
        id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        match prepaint.take() {
            Some(ReportedFrame::Image { image, fitted }) => {
                if let Some(global_id) = id {
                    paint_image_shadow(global_id, self.shadow, &image, fitted, window, cx);
                }
            }
            Some(ReportedFrame::Opaque { bounds, corner_radius }) => {
                let shadow = BoxShadow {
                    color: self.shadow.color,
                    offset: point(px(self.shadow.x), px(self.shadow.y)),
                    blur_radius: px(self.shadow.radius),
                    spread_radius: px(0.0),
                    inset: false,
                };
                window.paint_drop_shadows(bounds, Corners::all(px(corner_radius)), &[shadow]);
            }
            None => {}
        }
        self.content.paint(window, cx);
    }
}

fn paint_image_shadow(
    global_id: &GlobalElementId,
    shadow: AlphaShadow,
    image: &Arc<RenderImage>,
    fitted: Bounds<Pixels>,
    window: &mut Window,
    cx: &mut App,
) {
    let source_pixels = image.size(0);
    let pixels = (source_pixels.width.0 as usize, source_pixels.height.0 as usize);
    let sigma = source_sigma(
        shadow.radius,
        (pixels.0 as f32, pixels.1 as f32),
        (f32::from(fitted.size.width), f32::from(fitted.size.height)),
    );
    let plan = AlphaShadowPlan::new(sigma);
    let color = Rgba::from(shadow.color);
    let key: AlphaShadowKey = (
        image.id,
        (sigma * SIGMA_STEPS_PER_PIXEL).round() as u32,
        [channel(color.b), channel(color.g), channel(color.r), channel(color.a)],
    );
    let current = request_alpha_shadow(image, key, plan, window, cx);
    let shown = window.with_element_state(
        global_id,
        |last: Option<Option<ShownAlphaShadow>>, _window| {
            let last = last.flatten();
            let shown = match current {
                Some(baked) => Some((baked, ShownAlphaShadow { key, pixels, sigma })),
                None => last.and_then(|last| {
                    cx.default_global::<AlphaShadowCache>()
                        .ready(&last.key)
                        .map(|baked| (baked, last))
                }),
            };
            let remembered = shown.as_ref().map(|(_, memo)| *memo).or(last);
            (shown, remembered)
        },
    );
    let Some((baked, memo)) = shown else {
        return;
    };
    let target = alpha_shadow_bounds(
        fitted,
        memo.pixels,
        AlphaShadowPlan::new(memo.sigma),
        (shadow.x, shadow.y),
    );
    let _ = window.paint_image(target, target, Corners::default(), baked, 0, false);
}

fn channel(value: f32) -> u8 {
    (value.clamp(0.0, 1.0) * 255.0).round() as u8
}
