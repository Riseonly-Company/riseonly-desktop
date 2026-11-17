use std::cell::RefCell;
use std::collections::VecDeque;
use std::sync::Arc;

use gpui::{
    App, Background, Bounds, Canvas, ContentMask, Corners, Hsla, IntoElement, Path, Pixels, RenderImage, Rgba,
    Styled, Window, canvas, fill, linear_color_stop, linear_gradient, point, px,
};
use image::{Frame, RgbaImage};
use smallvec::smallvec;

use crate::swiftui::angular_gradient::{point_on_circle, sector_count};
use crate::swiftui::environment::{LayoutDirection, layout_direction};
use crate::swiftui::linear_gradient::gpui_gradient_for;
use crate::swiftui::transition::UnitPoint;

const OPAQUE_SEAM_OVERLAP: f32 = 0.2;

const OPAQUE_RING_OVERLAP: f32 = 0.5;

/// view that fills the space with colors changing by distance from center
/// first color up to start radius and last from end radius
#[derive(Clone, Debug, PartialEq)]
pub struct RadialGradient {
    pub colors: Vec<Hsla>,
    pub center: UnitPoint,
    pub start_radius: f32,
    pub end_radius: f32,
}

impl RadialGradient {
    /// radial gradient, colors spread evenly from start radius to end radius
    pub fn new(
        colors: impl IntoIterator<Item = Hsla>,
        center: UnitPoint,
        start_radius: f32,
        end_radius: f32,
    ) -> Self {
        Self {
            colors: colors.into_iter().collect(),
            center,
            start_radius,
            end_radius,
        }
    }

    /// the gradient as an element, fills its parent
    /// a fixed height is a size set on the returned canvas
    pub fn view(self) -> Canvas<LayoutDirection> {
        self.view_rounded(0.0)
    }

    /// the gradient as an element that fills its parent inside a rounded rect of corner_radius
    /// gpui clips children only by a rectangle, so a rounded card hands its radius to the gradient
    pub fn view_rounded(self, corner_radius: f32) -> Canvas<LayoutDirection> {
        canvas(
            |_, _, cx: &mut App| layout_direction(cx),
            move |bounds, direction, window, _| {
                self.paint_centered(bounds, self.center.resolved(direction), corner_radius, window)
            },
        )
        .size_full()
    }

    /// paints into bounds around the given center, left to right
    pub fn paint(&self, bounds: Bounds<Pixels>, window: &mut Window) {
        self.paint_centered(bounds, self.center, 0.0, window);
    }

    /// color at this fraction from start radius to end radius, neighbours blend in srgb
    pub fn color_at(&self, fraction: f32) -> Hsla {
        match self.colors.as_slice() {
            [] => Hsla::transparent_black(),
            [only] => *only,
            colors => {
                let last = colors.len() - 1;
                let position = fraction.clamp(0.0, 1.0) * last as f32;
                let index = (position.floor() as usize).min(last - 1);
                let local = position - index as f32;
                mix_srgb(colors[index], colors[index + 1], local)
            }
        }
    }

    /// color shown at this distance in points from the center
    pub fn color_at_distance(&self, distance: f32) -> Hsla {
        let span = self.end_radius - self.start_radius;
        if span <= f32::EPSILON {
            let fraction = if distance < self.start_radius { 0.0 } else { 1.0 };
            return self.color_at(fraction);
        }
        self.color_at((distance - self.start_radius) / span)
    }

    /// radius of color index, colors are spread evenly between the two radii
    pub fn stop_radius(&self, index: usize) -> f32 {
        let last = self.colors.len().saturating_sub(1);
        if last == 0 {
            return self.start_radius;
        }
        let span = (self.end_radius - self.start_radius).max(0.0);
        self.start_radius + span * index as f32 / last as f32
    }

    fn is_opaque(&self) -> bool {
        self.colors.iter().all(|color| color.a >= 1.0)
    }

    fn paint_centered(&self, bounds: Bounds<Pixels>, center: UnitPoint, corner_radius: f32, window: &mut Window) {
        let Some(first) = self.colors.first().copied() else {
            return;
        };
        let origin = (f32::from(bounds.origin.x), f32::from(bounds.origin.y));
        let size = (f32::from(bounds.size.width), f32::from(bounds.size.height));
        if size.0 <= 0.0 || size.1 <= 0.0 {
            return;
        }
        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            let corners = Corners::all(px(corner_radius.max(0.0).min(size.0.min(size.1) / 2.0)));
            if self.colors.len() == 1 {
                if corner_radius > 0.0 {
                    window.paint_quad(fill(bounds, first).corner_radii(corners));
                } else {
                    paint_rect(window, origin, size, first);
                }
                return;
            }
            if let Some(image) = RadialGradientRaster::image(self, center, size, window) {
                let _ = window.paint_image(bounds, bounds, corners, image, 0, false);
                return;
            }
            let geometry = RadialGeometry::new(origin, size, center, self);
            self.paint_rings(&geometry, window);
        });
    }

    fn paint_rings(&self, geometry: &RadialGeometry, window: &mut Window) {
        let ring_overlap = if self.is_opaque() {
            OPAQUE_RING_OVERLAP
        } else {
            0.0
        };
        let last_index = self.colors.len() - 1;

        let inner = self.stop_radius(0).max(0.0);
        if inner > 0.0 {
            geometry.paint_solid_ring(window, 0.0, inner + ring_overlap, self.colors[0]);
        }

        for index in 0..last_index {
            let from = self.stop_radius(index).max(0.0);
            let to = self.stop_radius(index + 1).max(0.0);
            if to - from <= f32::EPSILON || from >= geometry.far {
                continue;
            }
            geometry.paint_gradient_ring(
                window,
                (from, self.colors[index]),
                (to, self.colors[index + 1]),
                ring_overlap,
            );
        }

        let outer = self.stop_radius(last_index).max(0.0);
        let far = geometry.covering(geometry.far);
        if outer < far {
            geometry.paint_solid_ring(window, outer, far, self.colors[last_index]);
        }
    }
}

impl IntoElement for RadialGradient {
    type Element = Canvas<LayoutDirection>;

    fn into_element(self) -> Self::Element {
        self.view()
    }
}

fn mix_srgb(from: Hsla, to: Hsla, amount: f32) -> Hsla {
    let from = from.to_rgb();
    let to = to.to_rgb();
    let lerp = |a: f32, b: f32| a + (b - a) * amount;
    Hsla::from(Rgba {
        r: lerp(from.r, to.r),
        g: lerp(from.g, to.g),
        b: lerp(from.b, to.b),
        a: lerp(from.a, to.a),
    })
}

fn paint_rect(window: &mut Window, origin: (f32, f32), size: (f32, f32), color: Hsla) {
    let (x, y) = origin;
    let mut path = Path::new(point(px(x), px(y)));
    path.line_to(point(px(x + size.0), px(y)));
    path.line_to(point(px(x + size.0), px(y + size.1)));
    path.line_to(point(px(x), px(y + size.1)));
    window.paint_path(path, color);
}

/// what one rasterised gradient looks like, equal keys draw the same bitmap
#[derive(Clone, PartialEq, Eq)]
struct RadialGradientRasterKey {
    colors: Vec<[u32; 4]>,
    geometry: [u32; 6],
    pixels: (u32, u32),
}

enum RadialGradientRaster {}

thread_local! {
    static RADIAL_GRADIENT_RASTERS: RefCell<VecDeque<(RadialGradientRasterKey, Arc<RenderImage>)>> =
        const { RefCell::new(VecDeque::new()) };
}

impl RadialGradientRaster {
    const MAX_SIDE: f32 = 384.0;
    const CAPACITY: usize = 48;
    const STEPS: usize = 512;

    fn image(
        gradient: &RadialGradient,
        center: UnitPoint,
        size: (f32, f32),
        window: &mut Window,
    ) -> Option<Arc<RenderImage>> {
        let longest = size.0.max(size.1);
        if !longest.is_finite() || longest <= 0.0 {
            return None;
        }
        let scale = window.scale_factor().min(Self::MAX_SIDE / longest);
        if !scale.is_finite() || scale <= 0.0 {
            return None;
        }
        let width = ((size.0 * scale).ceil() as u32).max(1);
        let height = ((size.1 * scale).ceil() as u32).max(1);
        let key = RadialGradientRasterKey {
            colors: gradient
                .colors
                .iter()
                .map(|color| [color.h.to_bits(), color.s.to_bits(), color.l.to_bits(), color.a.to_bits()])
                .collect(),
            geometry: [
                center.x.to_bits(),
                center.y.to_bits(),
                gradient.start_radius.to_bits(),
                gradient.end_radius.to_bits(),
                size.0.to_bits(),
                size.1.to_bits(),
            ],
            pixels: (width, height),
        };
        let hit = RADIAL_GRADIENT_RASTERS.with_borrow_mut(|rasters| {
            let index = rasters.iter().position(|(cached, _)| *cached == key)?;
            let entry = rasters.remove(index)?;
            let image = entry.1.clone();
            rasters.push_front(entry);
            Some(image)
        });
        if hit.is_some() {
            return hit;
        }
        let image = Arc::new(RenderImage::new(smallvec![Frame::new(Self::draw(
            gradient, center, size, width, height
        )?)]));
        let evicted = RADIAL_GRADIENT_RASTERS.with_borrow_mut(|rasters| {
            rasters.push_front((key, image.clone()));
            if rasters.len() > Self::CAPACITY {
                rasters.pop_back().map(|(_, evicted)| evicted)
            } else {
                None
            }
        });
        if let Some(evicted) = evicted {
            let _ = window.drop_image(evicted);
        }
        Some(image)
    }

    /// gradient as straight alpha BGRA pixels, the order gpui uploads image frames in
    fn draw(gradient: &RadialGradient, center: UnitPoint, size: (f32, f32), width: u32, height: u32) -> Option<RgbaImage> {
        let lookup: Vec<[u8; 4]> = (0..Self::STEPS)
            .map(|step| {
                let color = gradient.color_at(step as f32 / (Self::STEPS - 1) as f32).to_rgb();
                let byte = |value: f32| (value.clamp(0.0, 1.0) * 255.0).round() as u8;
                [byte(color.b), byte(color.g), byte(color.r), byte(color.a)]
            })
            .collect();
        let span = gradient.end_radius - gradient.start_radius;
        let center_x = center.x * size.0;
        let center_y = center.y * size.1;
        let step_x = size.0 / width as f32;
        let step_y = size.1 / height as f32;
        let last = (Self::STEPS - 1) as f32;
        let mut pixels = Vec::with_capacity(width as usize * height as usize * 4);
        for row in 0..height {
            let dy = (row as f32 + 0.5) * step_y - center_y;
            for column in 0..width {
                let dx = (column as f32 + 0.5) * step_x - center_x;
                let distance = (dx * dx + dy * dy).sqrt();
                let fraction = if span <= f32::EPSILON {
                    if distance < gradient.start_radius { 0.0 } else { 1.0 }
                } else {
                    ((distance - gradient.start_radius) / span).clamp(0.0, 1.0)
                };
                pixels.extend_from_slice(&lookup[(fraction * last).round() as usize]);
            }
        }
        RgbaImage::from_raw(width, height, pixels)
    }
}

/// center, sector split and the radius past which nothing in the bounds is reached
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct RadialGeometry {
    pub(crate) center: (f32, f32),
    pub(crate) far: f32,
    pub(crate) sectors: usize,
    seam_overlap: f32,
}

impl RadialGeometry {
    pub(crate) fn new(
        origin: (f32, f32),
        size: (f32, f32),
        center: UnitPoint,
        gradient: &RadialGradient,
    ) -> Self {
        let center = (origin.0 + center.x * size.0, origin.1 + center.y * size.1);
        let far = farthest_corner(center, origin, size);
        let painted = gradient.end_radius.max(gradient.start_radius).clamp(0.0, far);
        let seam_overlap = if gradient.is_opaque() {
            OPAQUE_SEAM_OVERLAP
        } else {
            0.0
        };
        Self {
            center,
            far,
            sectors: sector_count(2.0 * painted),
            seam_overlap,
        }
    }

    fn step(&self) -> f32 {
        360.0 / self.sectors as f32
    }

    /// outer radius so a sector straight outer edge still reaches radius everywhere
    pub(crate) fn covering(&self, radius: f32) -> f32 {
        let half = (self.step() / 2.0 + self.seam_overlap).to_radians();
        radius / half.cos().max(f32::EPSILON)
    }

    /// all sectors between inner and outer in one color, as one path of disjoint quads
    fn paint_solid_ring(&self, window: &mut Window, inner: f32, outer: f32, color: Hsla) {
        let step = self.step();
        let mut path: Option<Path<Pixels>> = None;
        for sector in 0..self.sectors {
            let start = sector as f32 * step;
            let corners = self.sector_corners(inner, outer, start, start + step + self.seam_overlap);
            let contour = path.get_or_insert_with(|| Path::new(to_point(corners[0])));
            contour.move_to(to_point(corners[0]));
            for &corner in &corners[1..] {
                contour.line_to(to_point(corner));
            }
        }
        if let Some(path) = path {
            window.paint_path(path, color);
        }
    }

    fn paint_gradient_ring(
        &self,
        window: &mut Window,
        from: (f32, Hsla),
        to: (f32, Hsla),
        ring_overlap: f32,
    ) {
        let step = self.step();
        let to_color = to.1;
        let outer_radius = to.0 + ring_overlap;
        for sector in 0..self.sectors {
            let start = sector as f32 * step;
            let end = start + step + self.seam_overlap;
            let corners = self.sector_corners(from.0, outer_radius, start, end);
            let middle = start + step / 2.0;
            let background = sector_background(self.center, &corners, middle, from, to)
                .unwrap_or_else(|| mix_srgb(from.1, to_color, 0.5).into());
            let mut path = Path::new(to_point(corners[0]));
            for &corner in &corners[1..] {
                path.line_to(to_point(corner));
            }
            window.paint_path(path, background);
        }
    }

    pub(crate) fn sector_corners(
        &self,
        inner: f32,
        outer: f32,
        start: f32,
        end: f32,
    ) -> [(f32, f32); 4] {
        [
            point_on_circle(self.center, inner, start),
            point_on_circle(self.center, outer, start),
            point_on_circle(self.center, outer, end),
            point_on_circle(self.center, inner, end),
        ]
    }
}

/// distance from center to the farthest corner of the bounds
pub(crate) fn farthest_corner(center: (f32, f32), origin: (f32, f32), size: (f32, f32)) -> f32 {
    let dx = (center.0 - origin.0)
        .abs()
        .max((origin.0 + size.0 - center.0).abs());
    let dy = (center.1 - origin.1)
        .abs()
        .max((origin.1 + size.1 - center.1).abs());
    (dx * dx + dy * dy).sqrt()
}

fn to_point(p: (f32, f32)) -> gpui::Point<Pixels> {
    point(px(p.0), px(p.1))
}

fn sector_background(
    center: (f32, f32),
    corners: &[(f32, f32); 4],
    middle: f32,
    from: (f32, Hsla),
    to: (f32, Hsla),
) -> Option<Background> {
    let mut min = (f32::INFINITY, f32::INFINITY);
    let mut max = (f32::NEG_INFINITY, f32::NEG_INFINITY);
    for &(x, y) in corners {
        min = (min.0.min(x), min.1.min(y));
        max = (max.0.max(x), max.1.max(y));
    }
    let size = (max.0 - min.0, max.1 - min.1);
    let radians = middle.to_radians();
    let vector = (radians.cos(), radians.sin());
    let (angle, from_percentage, to_percentage) = gpui_gradient_for(
        min,
        size,
        vector,
        point_on_circle(center, from.0, middle),
        point_on_circle(center, to.0, middle),
    )?;
    Some(linear_gradient(
        angle,
        linear_color_stop(from.1, from_percentage),
        linear_color_stop(to.1, to_percentage),
    ))
}
