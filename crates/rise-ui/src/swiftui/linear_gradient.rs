use gpui::{
    App, Background, Bounds, Canvas, Hsla, IntoElement, Path, Pixels, Styled, Window, canvas,
    linear_color_stop, linear_gradient, point, px,
};

use crate::swiftui::environment::{LayoutDirection, layout_direction};
use crate::swiftui::transition::UnitPoint;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GradientStop {
    pub color: Hsla,
    pub location: f32,
}

impl GradientStop {
    pub fn new(color: impl Into<Hsla>, location: f32) -> Self {
        Self {
            color: color.into(),
            location,
        }
    }
}

/// view that fills the space with colors changing along the line from start to end
#[derive(Clone, Debug, PartialEq)]
pub struct LinearGradient {
    pub stops: Vec<GradientStop>,
    pub start_point: UnitPoint,
    pub end_point: UnitPoint,
}

impl LinearGradient {
    pub fn new(stops: Vec<GradientStop>, start_point: UnitPoint, end_point: UnitPoint) -> Self {
        Self {
            stops,
            start_point,
            end_point,
        }
    }

    /// gradient from colors spread evenly from 0 to 1
    pub fn colors(
        colors: impl IntoIterator<Item = Hsla>,
        start_point: UnitPoint,
        end_point: UnitPoint,
    ) -> Self {
        let colors: Vec<Hsla> = colors.into_iter().collect();
        let last = colors.len().saturating_sub(1);
        let stops = colors
            .into_iter()
            .enumerate()
            .map(|(index, color)| {
                let location = if last == 0 {
                    0.0
                } else {
                    index as f32 / last as f32
                };
                GradientStop { color, location }
            })
            .collect();
        Self::new(stops, start_point, end_point)
    }

    /// the gradient as an element, fills its parent
    /// a fixed height is a size set on the returned canvas
    pub fn view(self) -> Canvas<LayoutDirection> {
        canvas(
            |_, _, cx: &mut App| layout_direction(cx),
            move |bounds, direction, window, _| self.paint(bounds, direction, window),
        )
        .size_full()
    }

    /// paints into bounds, leading and trailing resolved for direction
    pub fn paint(&self, bounds: Bounds<Pixels>, direction: LayoutDirection, window: &mut Window) {
        let Some(first) = self.stops.first() else {
            return;
        };
        let origin = (f32::from(bounds.origin.x), f32::from(bounds.origin.y));
        let size = (f32::from(bounds.size.width), f32::from(bounds.size.height));
        let rect = GradientPolygon::rect(origin, size);
        if self.stops.len() == 1 {
            paint_polygon(window, &rect, first.color.into());
            return;
        }
        let Some(axis) = GradientAxis::new(
            origin,
            size,
            self.start_point.resolved(direction),
            self.end_point.resolved(direction),
        ) else {
            return;
        };

        let first_location = clamped_location(first.location, 0.0);
        let before = rect.clip(&axis, None, Some(axis.distance(first_location)));
        paint_polygon(window, &before, first.color.into());

        let mut previous = (first.color, first_location);
        for stop in &self.stops[1..] {
            let location = clamped_location(stop.location, previous.1);
            let from = axis.distance(previous.1);
            let to = axis.distance(location);
            let band = rect.clip(&axis, Some(from), Some(to));
            let background = band_background(&band, &axis, (previous.0, from), (stop.color, to));
            if let Some(background) = background {
                paint_polygon(window, &band, background);
            }
            previous = (stop.color, location);
        }

        let after = rect.clip(&axis, Some(axis.distance(previous.1)), None);
        paint_polygon(window, &after, previous.0.into());
    }
}

impl IntoElement for LinearGradient {
    type Element = Canvas<LayoutDirection>;

    fn into_element(self) -> Self::Element {
        self.view()
    }
}

/// core graphics wants ascending locations in 0 to 1
/// a stop before its neighbour is drawn at the neighbour so no band gets negative width
fn clamped_location(location: f32, previous: f32) -> f32 {
    location.clamp(0.0, 1.0).max(previous)
}

/// start to end line in window pixels
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct GradientAxis {
    start: (f32, f32),
    vector: (f32, f32),
    unit: (f32, f32),
    length: f32,
}

impl GradientAxis {
    /// none for a zero length line, no direction to shade so nothing is drawn
    pub(crate) fn new(
        origin: (f32, f32),
        size: (f32, f32),
        start: UnitPoint,
        end: UnitPoint,
    ) -> Option<Self> {
        let start = (origin.0 + start.x * size.0, origin.1 + start.y * size.1);
        let end = (origin.0 + end.x * size.0, origin.1 + end.y * size.1);
        let vector = (end.0 - start.0, end.1 - start.1);
        let length = (vector.0 * vector.0 + vector.1 * vector.1).sqrt();
        if !length.is_finite() || length < f32::EPSILON {
            return None;
        }
        Some(Self {
            start,
            vector,
            unit: (vector.0 / length, vector.1 / length),
            length,
        })
    }

    /// distance along the line from the start to the iso line of location
    pub(crate) fn distance(&self, location: f32) -> f32 {
        location * self.length
    }

    /// distance along the line from the start to the iso line through p
    pub(crate) fn project(&self, p: (f32, f32)) -> f32 {
        (p.0 - self.start.0) * self.unit.0 + (p.1 - self.start.1) * self.unit.1
    }

    /// point on the line at distance from the start
    pub(crate) fn point_at(&self, distance: f32) -> (f32, f32) {
        (
            self.start.0 + self.unit.0 * distance,
            self.start.1 + self.unit.1 * distance,
        )
    }
}

const MAX_CORNERS: usize = 8;

/// convex polygon in window pixels, no allocation
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct GradientPolygon {
    pub(crate) points: [(f32, f32); MAX_CORNERS],
    pub(crate) len: usize,
}

impl GradientPolygon {
    pub(crate) fn rect(origin: (f32, f32), size: (f32, f32)) -> Self {
        let (x, y) = origin;
        let (width, height) = size;
        let mut points = [(0.0, 0.0); MAX_CORNERS];
        points[0] = (x, y);
        points[1] = (x + width, y);
        points[2] = (x + width, y + height);
        points[3] = (x, y + height);
        Self { points, len: 4 }
    }

    fn corners(&self) -> &[(f32, f32)] {
        &self.points[..self.len]
    }

    /// part between the iso lines at from and to along axis, an open side is none
    pub(crate) fn clip(&self, axis: &GradientAxis, from: Option<f32>, to: Option<f32>) -> Self {
        let mut polygon = *self;
        if let Some(from) = from {
            polygon = polygon.keep(|p| axis.project(p) - from);
        }
        if let Some(to) = to {
            polygon = polygon.keep(|p| to - axis.project(p));
        }
        polygon
    }

    /// sutherland hodgman against one half plane, keeps corners where side is not negative
    fn keep(&self, side: impl Fn((f32, f32)) -> f32) -> Self {
        let mut out = Self {
            points: [(0.0, 0.0); MAX_CORNERS],
            len: 0,
        };
        let corners = self.corners();
        for (index, &current) in corners.iter().enumerate() {
            let next = corners[(index + 1) % corners.len()];
            let (current_side, next_side) = (side(current), side(next));
            if current_side >= 0.0 {
                out.push(current);
            }
            if (current_side >= 0.0) != (next_side >= 0.0) {
                let t = current_side / (current_side - next_side);
                out.push((
                    current.0 + (next.0 - current.0) * t,
                    current.1 + (next.1 - current.1) * t,
                ));
            }
        }
        out
    }

    fn push(&mut self, p: (f32, f32)) {
        if self.len < MAX_CORNERS {
            self.points[self.len] = p;
            self.len += 1;
        }
    }

    /// origin and size of the bounding box, gpui uses it as the path gradient box
    pub(crate) fn bounding_box(&self) -> ((f32, f32), (f32, f32)) {
        let corners = self.corners();
        let mut min = (f32::INFINITY, f32::INFINITY);
        let mut max = (f32::NEG_INFINITY, f32::NEG_INFINITY);
        for &(x, y) in corners {
            min = (min.0.min(x), min.1.min(y));
            max = (max.0.max(x), max.1.max(y));
        }
        (min, (max.0 - min.0, max.1 - min.1))
    }

    fn has_area(&self) -> bool {
        let ((_, _), (width, height)) = self.bounding_box();
        self.len >= 3 && width > f32::EPSILON && height > f32::EPSILON
    }
}

fn paint_polygon(window: &mut Window, polygon: &GradientPolygon, background: Background) {
    if !polygon.has_area() {
        return;
    }
    let corners = polygon.corners();
    let mut path = Path::new(point(px(corners[0].0), px(corners[0].1)));
    for &(x, y) in &corners[1..] {
        path.line_to(point(px(x), px(y)));
    }
    window.paint_path(path, background);
}

/// two stop gpui gradient for a band from one distance to another, none if the band has no width
pub(crate) fn band_background(
    band: &GradientPolygon,
    axis: &GradientAxis,
    from: (Hsla, f32),
    to: (Hsla, f32),
) -> Option<Background> {
    if !band.has_area() || to.1 - from.1 < f32::EPSILON {
        return None;
    }
    let (origin, size) = band.bounding_box();
    let (angle, from_percentage, to_percentage) =
        gpui_gradient_for(origin, size, axis.vector, axis.point_at(from.1), axis.point_at(to.1))?;
    Some(linear_gradient(
        angle,
        linear_color_stop(from.0, from_percentage),
        linear_color_stop(to.0, to_percentage),
    ))
}

/// angle in degrees and stop percentages so gpui gradient over the box runs along vector
/// at 0 through from and at 1 through to, gpui shrinks the short side by the box aspect
pub(crate) fn gpui_gradient_for(
    origin: (f32, f32),
    size: (f32, f32),
    vector: (f32, f32),
    from: (f32, f32),
    to: (f32, f32),
) -> Option<(f32, f32, f32)> {
    let (width, height) = size;
    if width <= f32::EPSILON || height <= f32::EPSILON {
        return None;
    }
    let direction = (vector.1 * width).atan2(vector.0 * height);
    let angle = (direction.to_degrees() + 90.0).rem_euclid(360.0);
    let from_percentage = gpui_gradient_parameter(angle, origin, size, from);
    let to_percentage = gpui_gradient_parameter(angle, origin, size, to);
    let span = to_percentage - from_percentage;
    if !span.is_finite() || span.abs() < f32::EPSILON {
        return None;
    }
    Some((angle, from_percentage, to_percentage))
}

/// gpui gradient parameter at p before the stop percentages, same steps as its fragment shader
pub(crate) fn gpui_gradient_parameter(
    angle: f32,
    origin: (f32, f32),
    size: (f32, f32),
    p: (f32, f32),
) -> f32 {
    let (width, height) = size;
    let radians = ((angle % 360.0) - 90.0).to_radians();
    let (mut x, mut y) = (radians.cos(), radians.sin());
    if width > height {
        y *= height / width;
    } else {
        x *= width / height;
    }
    let half = (width / 2.0, height / 2.0);
    let centre = (origin.0 + half.0, origin.1 + half.1);
    let along = ((p.0 - centre.0) * x + (p.1 - centre.1) * y) / (x * x + y * y).sqrt();
    if x.abs() > y.abs() {
        (along + half.0) / width
    } else {
        (along + half.1) / height
    }
}
