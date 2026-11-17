use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

use gpui::{
    AnyElement, App, Background, Bounds, ContentMask, Context, Div, Element, ElementId, Entity, EntityId,
    GlobalElementId, Hsla, InspectorElementId, IntoElement, LayoutId, ParentElement, Path as GpuiPath, Pixels, Point,
    RenderOnce, Styled, Subscription, Window, canvas, div, fill, linear_color_stop, linear_gradient, point,
    prelude::FluentBuilder, px,
};
use rise_core::foundation::foundation_date::RiseClock;
use rise_energy::rise_energy_saving::{RiseEnergySavingFeature, RiseEnergySavingStore};
use rise_theme::AppTheme;

use crate::animated_media_playback_scope::is_animated_media_playback_active;
use crate::frame_request::{CAPPED_TICK_INTERVAL, is_shown_on_screen};
use crate::swiftui::environment::{EnvKey, ScenePhase, env, env_scope, reduce_motion, scene_phase};
use crate::swiftui::frame::{EdgeSet, SwiftUiFrame};
use crate::swiftui::linear_gradient::gpui_gradient_for;
use crate::swiftui::shapes::Shape;
use crate::swiftui::stacks::{HorizontalAlignment, StackAlign, hstack, spacer_min_w, vstack};

pub struct SkeletonUi;

impl SkeletonUi {
    pub const SHIMMER_BAND_LENGTH: f32 = 250.0;
    pub const SHIMMER_DURATION: f64 = 1.3;
    pub const SHIMMER_MAX_RUN: Duration = Duration::from_secs(8);

    pub fn shape_color(theme: &AppTheme) -> Hsla {
        theme.bg._300
    }

    pub fn shimmer_peak_color(theme: &AppTheme) -> Hsla {
        theme.bg._100.opacity(0.4)
    }
}

#[derive(IntoElement)]
pub struct SkeletonCircle {
    pub size: f32,
}

impl SkeletonCircle {
    pub fn new(size: f32) -> Self {
        Self { size }
    }
}

impl RenderOnce for SkeletonCircle {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        skeleton_shape(Shape::Circle, cx).size(px(self.size)).flex_none()
    }
}

#[derive(IntoElement)]
pub struct SkeletonLine {
    pub width: Option<f32>,
    pub diameter: f32,
}

impl Default for SkeletonLine {
    fn default() -> Self {
        Self { width: None, diameter: 10.0 }
    }
}

impl SkeletonLine {
    pub fn new(width: impl Into<Option<f32>>, diameter: f32) -> Self {
        Self { width: width.into(), diameter }
    }
}

impl RenderOnce for SkeletonLine {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let line = skeleton_shape(Shape::Capsule, cx).frame_size(self.width, self.diameter);
        if self.width.is_none() { line.w_full() } else { line.flex_none() }
    }
}

#[derive(IntoElement)]
pub struct SkeletonBlock {
    pub width: Option<f32>,
    pub height: Option<f32>,
    pub corner_radius: f32,
}

impl Default for SkeletonBlock {
    fn default() -> Self {
        Self { width: None, height: None, corner_radius: 10.0 }
    }
}

impl SkeletonBlock {
    pub fn new(width: impl Into<Option<f32>>, height: impl Into<Option<f32>>, corner_radius: f32) -> Self {
        Self { width: width.into(), height: height.into(), corner_radius }
    }
}

impl RenderOnce for SkeletonBlock {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let block = skeleton_shape(Shape::continuous(self.corner_radius), cx).frame_size(self.width, self.height);
        let block = if self.width.is_none() { block.w_full() } else { block };
        if self.height.is_none() { block.h_full() } else { block }
    }
}

/// filled shape plus the band drawn inside its outline under a running skeleton_shimmer
fn skeleton_shape(shape: Shape, cx: &App) -> Div {
    let band = env::<SkeletonShimmerKey>(cx);
    shape
        .round(div().relative().bg(SkeletonUi::shape_color(crate::theme(cx))))
        .when_some(band, |element, band| element.child(skeleton_shimmer_highlight(shape, band)))
}

#[derive(IntoElement)]
pub struct SkeletonListUi {
    pub rows: usize,
    pub avatar_size: f32,
}

impl Default for SkeletonListUi {
    fn default() -> Self {
        Self { rows: 8, avatar_size: 40.0 }
    }
}

impl SkeletonListUi {
    const TITLE_WIDTHS: [f32; 4] = [140.0, 100.0, 170.0, 120.0];
    const SUBTITLE_WIDTHS: [f32; 4] = [90.0, 130.0, 70.0, 110.0];

    pub fn new(rows: usize, avatar_size: f32) -> Self {
        Self { rows, avatar_size }
    }
}

impl RenderOnce for SkeletonListUi {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let avatar_size = self.avatar_size;
        let rows = (0..self.rows).map(move |index| {
            hstack(10.0)
                .w_full()
                .child(SkeletonCircle::new(avatar_size))
                .child(
                    vstack(8.0)
                        .align(HorizontalAlignment::Leading)
                        .child(SkeletonLine::new(Self::TITLE_WIDTHS[index % Self::TITLE_WIDTHS.len()], 10.0))
                        .child(SkeletonLine::new(Self::SUBTITLE_WIDTHS[index % Self::SUBTITLE_WIDTHS.len()], 8.0)),
                )
                .child(spacer_min_w(0.0))
                .padding_edges(EdgeSet::HORIZONTAL, 16.0)
                .padding_edges(EdgeSet::VERTICAL, 10.0)
        });
        vstack(0.0).w_full().children(rows).skeleton_shimmer(None)
    }
}

pub trait SkeletonShimmerExt: IntoElement + Sized {
    fn skeleton_shimmer(self, surface: Option<Hsla>) -> SkeletonShimmerModifier {
        SkeletonShimmerModifier { surface, content: self.into_any_element() }
    }
}

impl<E: IntoElement> SkeletonShimmerExt for E {}

/// where the shimmer band sits in the window this frame and the color of its middle
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SkeletonShimmerBand {
    pub peak: Hsla,
    pub top_in_window: f32,
    pub swap_base: u64,
}

impl SkeletonShimmerBand {
    /// band at monotonic time now sweeping a window sweep_height tall
    pub fn at(now: Duration, sweep_height: f32, peak: Hsla) -> Self {
        let duration = SkeletonUi::SHIMMER_DURATION;
        let progress = (now.as_secs_f64() % duration) / duration;
        let eased = 1.0 - (1.0 - progress) * (1.0 - progress);
        let band_length = f64::from(SkeletonUi::SHIMMER_BAND_LENGTH);
        let top = -band_length + (f64::from(sweep_height) + band_length) * eased;
        Self { peak, top_in_window: top as f32, swap_base: 0 }
    }

    pub fn with_swap_base(mut self, swap_base: u64) -> Self {
        self.swap_base = swap_base;
        self
    }

    /// paints the part of the band that falls inside shape laid out at bounds
    pub fn paint(self, bounds: Bounds<Pixels>, shape: Shape, window: &mut Window) {
        let radius = corner_radius(shape, bounds);
        if self.swap_base == 0 {
            let middle = self.top_in_window + SkeletonUi::SHIMMER_BAND_LENGTH / 2.0;
            let bottom = self.top_in_window + SkeletonUi::SHIMMER_BAND_LENGTH;
            let clear = self.peak.opacity(0.0);
            paint_ramp(bounds, radius, (self.top_in_window, clear), (middle, self.peak), window);
            paint_ramp(bounds, radius, (middle, self.peak), (bottom, clear), window);
            return;
        }
        let mask = window.content_mask().bounds;
        let visible = bounds.intersect(&mask);
        if visible.size.width <= Pixels::ZERO || visible.size.height <= Pixels::ZERO {
            return;
        }
        let shimmer_shape = ShimmerShape { bounds, radius: f32::from(radius), mask };
        let index = SHIMMER_SHAPES.with(|shapes| {
            let mut shapes = shapes.borrow_mut();
            let painted = shapes.entry(self.swap_base).or_default();
            painted.push(shimmer_shape);
            painted.len() - 1
        });
        for (half, (path, color)) in shimmer_shape.ramps(Some(self), self.peak, bounds.origin).into_iter().enumerate() {
            window.paint_swappable_path(path, color, shimmer_swap_key(self.swap_base, index * 2 + half), bounds.origin);
        }
    }
}

thread_local! {
    static SHIMMER_SHAPES: RefCell<HashMap<u64, Vec<ShimmerShape>>> = RefCell::new(HashMap::new());
}

fn shimmer_swap_key(swap_base: u64, index: usize) -> u64 {
    (swap_base.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ (index as u64 + 1)).max(1)
}

/// one skeleton shape as painted, where and how round and the clip it was painted under
#[derive(Clone, Copy)]
struct ShimmerShape {
    bounds: Bounds<Pixels>,
    radius: f32,
    mask: Bounds<Pixels>,
}

impl ShimmerShape {
    const CORNER_SEGMENTS: usize = 8;

    /// two halves of the band inside the shape from the shape origin
    /// a half that misses the shape or no band keeps the whole outline with a clear fill so its
    /// swap key stays in the scene
    fn ramps(
        &self,
        band: Option<SkeletonShimmerBand>,
        peak: Hsla,
        offset: Point<Pixels>,
    ) -> [(GpuiPath<Pixels>, Background); 2] {
        let clear = peak.opacity(0.0);
        let Some(band) = band else {
            return [self.clear_ramp(clear, offset), self.clear_ramp(clear, offset)];
        };
        let middle = band.top_in_window + SkeletonUi::SHIMMER_BAND_LENGTH / 2.0;
        let bottom = band.top_in_window + SkeletonUi::SHIMMER_BAND_LENGTH;
        [
            self.ramp((band.top_in_window, clear), (middle, band.peak), offset),
            self.ramp((middle, band.peak), (bottom, clear), offset),
        ]
    }

    fn clear_ramp(&self, clear: Hsla, offset: Point<Pixels>) -> (GpuiPath<Pixels>, Background) {
        let outline = self.outline();
        (Self::polygon_path(&outline, offset), clear.into())
    }

    fn ramp(&self, from: (f32, Hsla), to: (f32, Hsla), offset: Point<Pixels>) -> (GpuiPath<Pixels>, Background) {
        let clear = from.1.opacity(0.0);
        let top = f32::from(self.bounds.top());
        let start = from.0.max(top);
        let end = to.0.min(f32::from(self.bounds.bottom()));
        if end <= start {
            return self.clear_ramp(clear, offset);
        }
        let local_start = start - top;
        let local_end = end - top;
        let cut = Self::clip_rows(&self.outline(), local_start, local_end);
        if cut.len() < 3 {
            return self.clear_ramp(clear, offset);
        }
        let shaded_top = start.max(f32::from(self.mask.top()));
        let shaded_bottom = end.min(f32::from(self.mask.bottom()));
        if shaded_bottom <= shaded_top {
            return self.clear_ramp(clear, offset);
        }
        let left = f32::from(self.bounds.left());
        let Some((angle, from_percentage, to_percentage)) = gpui_gradient_for(
            (left, shaded_top),
            (f32::from(self.bounds.size.width), shaded_bottom - shaded_top),
            (0.0, 1.0),
            (left, from.0),
            (left, to.0),
        ) else {
            return self.clear_ramp(clear, offset);
        };
        let background =
            linear_gradient(angle, linear_color_stop(from.1, from_percentage), linear_color_stop(to.1, to_percentage));
        (Self::polygon_path(&cut, offset), background)
    }

    /// rounded outline from the shape origin, clockwise
    fn outline(&self) -> Vec<(f32, f32)> {
        let width = f32::from(self.bounds.size.width);
        let height = f32::from(self.bounds.size.height);
        let radius = self.radius.min(width / 2.0).min(height / 2.0).max(0.0);
        if radius <= 0.0 {
            return vec![(0.0, 0.0), (width, 0.0), (width, height), (0.0, height)];
        }
        let corners = [
            (width - radius, radius, -std::f32::consts::FRAC_PI_2),
            (width - radius, height - radius, 0.0),
            (radius, height - radius, std::f32::consts::FRAC_PI_2),
            (radius, radius, std::f32::consts::PI),
        ];
        let mut points = Vec::with_capacity(4 * (Self::CORNER_SEGMENTS + 1));
        for (center_x, center_y, start_angle) in corners {
            for step in 0..=Self::CORNER_SEGMENTS {
                let angle = start_angle + std::f32::consts::FRAC_PI_2 * step as f32 / Self::CORNER_SEGMENTS as f32;
                points.push((center_x + radius * angle.cos(), center_y + radius * angle.sin()));
            }
        }
        points
    }

    fn clip_rows(outline: &[(f32, f32)], start: f32, end: f32) -> Vec<(f32, f32)> {
        let below_start = Self::clip_half(outline, |y| y >= start, start);
        Self::clip_half(&below_start, |y| y <= end, end)
    }

    fn clip_half(points: &[(f32, f32)], inside: impl Fn(f32) -> bool, edge: f32) -> Vec<(f32, f32)> {
        let mut clipped = Vec::with_capacity(points.len() + 2);
        for (index, &current) in points.iter().enumerate() {
            let previous = points[(index + points.len() - 1) % points.len()];
            let crosses = |a: (f32, f32), b: (f32, f32)| {
                let t = (edge - a.1) / (b.1 - a.1);
                (a.0 + (b.0 - a.0) * t, edge)
            };
            match (inside(previous.1), inside(current.1)) {
                (true, true) => clipped.push(current),
                (true, false) => clipped.push(crosses(previous, current)),
                (false, true) => {
                    clipped.push(crosses(previous, current));
                    clipped.push(current);
                }
                (false, false) => {}
            }
        }
        clipped
    }

    /// fan of the convex polygon moved by offset, the window origin when painted and zero for a swap
    /// gpui multisamples path edges
    fn polygon_path(points: &[(f32, f32)], offset: Point<Pixels>) -> GpuiPath<Pixels> {
        let at = |(x, y): (f32, f32)| point(offset.x + px(x), offset.y + px(y));
        let first = points.first().copied().unwrap_or((0.0, 0.0));
        let mut path = GpuiPath::new(at(first));
        for &corner in points.iter().skip(1) {
            path.line_to(at(corner));
        }
        path
    }
}

/// the running band a skeleton_shimmer hands to the shapes inside it
pub struct SkeletonShimmerKey;

impl EnvKey for SkeletonShimmerKey {
    type Value = Option<SkeletonShimmerBand>;

    fn default_value() -> Self::Value {
        None
    }
}

pub fn skeleton_shimmer_highlight(shape: Shape, band: SkeletonShimmerBand) -> impl IntoElement {
    canvas(|_, _, _| (), move |bounds, (), window, _| band.paint(bounds, shape, window)).absolute().inset_0()
}

fn corner_radius(shape: Shape, bounds: Bounds<Pixels>) -> Pixels {
    match shape {
        Shape::Rectangle => px(0.0),
        Shape::RoundedRectangle { corner_radius, .. } => px(corner_radius),
        Shape::Capsule | Shape::Circle => px(f32::from(bounds.size.width.min(bounds.size.height)) / 2.0),
    }
}

/// one half of the band, the shape own rounded quad with a two stop gradient cut to the rows between
/// gpui masks only with rects and a horizontal cut keeps the outline exact
fn paint_ramp(bounds: Bounds<Pixels>, radius: Pixels, from: (f32, Hsla), to: (f32, Hsla), window: &mut Window) {
    let shape_top = f32::from(bounds.top());
    let start = from.0.max(shape_top);
    let end = to.0.min(f32::from(bounds.bottom()));
    if end <= start {
        return;
    }
    let origin = (f32::from(bounds.origin.x), shape_top);
    let size = (f32::from(bounds.size.width), f32::from(bounds.size.height));
    let Some((angle, from_percentage, to_percentage)) =
        gpui_gradient_for(origin, size, (0.0, 1.0), (origin.0, from.0), (origin.0, to.0))
    else {
        return;
    };
    let background =
        linear_gradient(angle, linear_color_stop(from.1, from_percentage), linear_color_stop(to.1, to_percentage));
    let rows = Bounds::from_corners(point(bounds.origin.x, px(start)), point(bounds.right(), px(end)));
    window.with_content_mask(Some(ContentMask { bounds: rows }), |window| {
        window.paint_quad(fill(bounds, background).corner_radii(radius));
    });
}

#[derive(IntoElement)]
pub struct SkeletonShimmerModifier {
    surface: Option<Hsla>,
    content: AnyElement,
}

/// observes the energy saving store, plus the swap loop that moves the painted band between renders
struct SkeletonShimmerState {
    started: Duration,
    _energy_changes: Subscription,
    swap_base: u64,
    sweep_height: f32,
    peak: Hsla,
    host: Option<EntityId>,
    swaps_running: bool,
    tick_queued: bool,
    loop_generation: u64,
}

impl SkeletonShimmerState {
    fn new(cx: &mut Context<Self>) -> Self {
        let energy = RiseEnergySavingStore::shared(cx);
        let energy_changes = cx.observe(&energy, |_, _, cx| cx.notify());
        Self {
            started: RiseClock::monotonic(),
            _energy_changes: energy_changes,
            swap_base: 0,
            sweep_height: 0.0,
            peak: Hsla::default(),
            host: None,
            swaps_running: false,
            tick_queued: false,
            loop_generation: 0,
        }
    }

    fn run_swaps(state: Entity<Self>, generation: u64, window: &Window, cx: &App) {
        window
            .spawn(cx, async move |cx| {
                loop {
                    cx.background_executor().timer(CAPPED_TICK_INTERVAL).await;
                    let keeps_running = cx
                        .update(|window, cx| {
                            let shimmer = state.read(cx);
                            let running = shimmer.swaps_running && shimmer.loop_generation == generation;
                            if running && !shimmer.tick_queued {
                                state.update(cx, |shimmer, _| shimmer.tick_queued = true);
                                let state = state.clone();
                                window.on_next_frame(move |window, cx| {
                                    state.update(cx, |shimmer, _| shimmer.tick_queued = false);
                                    Self::swap_tick(&state, window, cx);
                                });
                            }
                            running
                        })
                        .unwrap_or(false);
                    if !keeps_running {
                        break;
                    }
                }
            })
            .detach();
    }

    fn swap_tick(state: &Entity<Self>, window: &mut Window, cx: &mut App) {
        let shimmer = state.read(cx);
        let (swap_base, sweep_height, peak, host) = (shimmer.swap_base, shimmer.sweep_height, shimmer.peak, shimmer.host);
        let now = RiseClock::monotonic();
        let stops = now.saturating_sub(shimmer.started) >= SkeletonUi::SHIMMER_MAX_RUN
            || reduce_motion(cx)
            || RiseEnergySavingStore::snapshot_reduces(RiseEnergySavingFeature::InterfaceEffects);
        let band = (!stops).then(|| SkeletonShimmerBand::at(now, sweep_height, peak).with_swap_base(swap_base));
        let shapes = SHIMMER_SHAPES.with(|shapes| shapes.borrow().get(&swap_base).cloned()).unwrap_or_default();
        let swaps: Vec<(u64, GpuiPath<Pixels>, Background)> = shapes
            .iter()
            .enumerate()
            .flat_map(|(index, shape)| {
                shape
                    .ramps(band, peak, Point::default())
                    .into_iter()
                    .enumerate()
                    .map(move |(half, (path, color))| (shimmer_swap_key(swap_base, index * 2 + half), path, color))
            })
            .collect();
        let swapped = !swaps.is_empty() && window.swap_paths(swaps);
        if swapped && !stops {
            return;
        }
        state.update(cx, |shimmer, _| shimmer.swaps_running = false);
        if !swapped
            && !stops
            && !shapes.is_empty()
            && let Some(host) = host
        {
            cx.notify(host);
        }
    }
}

impl Drop for SkeletonShimmerState {
    fn drop(&mut self) {
        let swap_base = self.swap_base;
        let _ = SHIMMER_SHAPES.try_with(|shapes| shapes.borrow_mut().remove(&swap_base));
    }
}

impl RenderOnce for SkeletonShimmerModifier {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = window.use_keyed_state(ElementId::Name("skeleton_shimmer".into()), cx, |_, cx| {
            SkeletonShimmerState::new(cx)
        });
        let now = RiseClock::monotonic();
        let running = now.saturating_sub(state.read(cx).started) < SkeletonUi::SHIMMER_MAX_RUN;
        let energy = RiseEnergySavingStore::shared(cx);
        let (low_power, reduces_effects) = {
            let energy = energy.read(cx);
            (energy.system_low_power_mode(), energy.reduces(RiseEnergySavingFeature::InterfaceEffects))
        };
        let should_animate = scene_phase(cx) == ScenePhase::Active
            && is_animated_media_playback_active(cx)
            && !reduce_motion(cx)
            && !low_power
            && !reduces_effects
            && running;
        let swap_base = state.entity_id().as_u64();
        SHIMMER_SHAPES.with(|shapes| shapes.borrow_mut().remove(&swap_base));
        if !should_animate {
            state.update(cx, |shimmer, _| shimmer.swaps_running = false);
            return self.content;
        }

        let peak = self
            .surface
            .map(|surface| surface.opacity(0.4))
            .unwrap_or_else(|| SkeletonUi::shimmer_peak_color(crate::theme(cx)));
        let sweep_height = f32::from(window.viewport_size().height);
        let band = SkeletonShimmerBand::at(now, sweep_height, peak).with_swap_base(swap_base);
        state.update(cx, |shimmer, _| {
            shimmer.swap_base = swap_base;
            shimmer.sweep_height = sweep_height;
            shimmer.peak = peak;
        });
        SkeletonOnScreenGate {
            content: env_scope::<SkeletonShimmerKey>(Some(band)).child(self.content).into_any_element(),
            state,
        }
        .into_any_element()
    }
}

struct SkeletonOnScreenGate {
    content: AnyElement,
    state: Entity<SkeletonShimmerState>,
}

impl IntoElement for SkeletonOnScreenGate {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for SkeletonOnScreenGate {
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
        if is_shown_on_screen(bounds, window) {
            let host = window.current_view();
            let started = self.state.update(cx, |shimmer, _| {
                shimmer.host = Some(host);
                if shimmer.swaps_running {
                    return None;
                }
                shimmer.swaps_running = true;
                shimmer.loop_generation = shimmer.loop_generation.wrapping_add(1);
                Some(shimmer.loop_generation)
            });
            if let Some(generation) = started {
                SkeletonShimmerState::run_swaps(self.state.clone(), generation, window, cx);
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

/// true when the bounds meet the content mask of every clipping ancestor and the window
fn is_on_screen(bounds: Bounds<Pixels>, window: &Window) -> bool {
    let clip = window.content_mask().bounds;
    if !bounds.intersects(&clip) {
        return false;
    }
    let visible = bounds.intersect(&clip);
    visible.size.width > px(1.0) && visible.size.height > Pixels::ZERO
}

/// also the on screen gate for other looping effects (the first comment wave)
/// a timeline on a mounted but offscreen row or pager page pauses through on_visibility
/// mount it as the effect background
#[derive(IntoElement)]
pub struct SkeletonWindowHeightReader {
    id: ElementId,
    on_height: Rc<dyn Fn(f32, &mut Window, &mut App)>,
    on_visibility: Rc<dyn Fn(bool, &mut Window, &mut App)>,
}

impl SkeletonWindowHeightReader {
    /// id keys the last reported values across frames
    pub fn new(id: impl Into<ElementId>, on_visibility: impl Fn(bool, &mut Window, &mut App) + 'static) -> Self {
        Self { id: id.into(), on_height: Rc::new(|_: f32, _: &mut Window, _: &mut App| {}), on_visibility: Rc::new(on_visibility) }
    }

    pub fn on_height(mut self, on_height: impl Fn(f32, &mut Window, &mut App) + 'static) -> Self {
        self.on_height = Rc::new(on_height);
        self
    }
}

#[derive(Default)]
struct SkeletonWindowHeightReaderProbe {
    last_height: Option<f32>,
    last_visible: Option<bool>,
}

impl RenderOnce for SkeletonWindowHeightReader {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let probe = window.use_keyed_state(self.id, cx, |_, _| SkeletonWindowHeightReaderProbe::default());
        let on_height = self.on_height;
        let on_visibility = self.on_visibility;
        canvas(
            move |bounds, window, cx| {
                let height = f32::from(window.viewport_size().height);
                let visible = is_on_screen(bounds, window);
                let (height_changed, visibility_changed) = probe.update(cx, |probe, _| {
                    let height_changed = height > 0.0 && probe.last_height != Some(height);
                    let visibility_changed = probe.last_visible != Some(visible);
                    if height_changed {
                        probe.last_height = Some(height);
                    }
                    probe.last_visible = Some(visible);
                    (height_changed, visibility_changed)
                });
                if height_changed {
                    window.defer(cx, move |window, cx| on_height(height, window, cx));
                }
                if visibility_changed {
                    window.defer(cx, move |window, cx| on_visibility(visible, window, cx));
                }
            },
            |_, (), _, _| {},
        )
        .absolute()
        .inset_0()
    }
}
