use gpui::{Background, Bounds, Canvas, Hsla, Path, Pixels, Styled, Window, canvas, point, px};

use crate::swiftui::environment::{LayoutDirection, layout_direction};
use crate::swiftui::linear_gradient::{GradientAxis, GradientPolygon, LinearGradient, band_background};

const CORNER_SEGMENTS: usize = 8;

type GradientPoint = (f32, f32);

/// rounded rect filled with a gradient, fills its parent
pub fn rounded_gradient_fill(gradient: LinearGradient, corner_radius: f32) -> Canvas<LayoutDirection> {
    canvas(
        |_, _, cx| layout_direction(cx),
        move |bounds, direction, window, _| {
            let (origin, size) = rect_of(bounds);
            let pieces = vec![rounded_outline(origin, size, corner_radius)];
            paint_pieces(&gradient, &pieces, bounds, direction, window);
        },
    )
    .size_full()
}

/// rounded rect border stroked with a gradient, fills its parent
/// the stroke lies wholly inside the outline
pub fn rounded_gradient_stroke_border(
    gradient: LinearGradient,
    corner_radius: f32,
    line_width: f32,
) -> Canvas<LayoutDirection> {
    canvas(
        |_, _, cx| layout_direction(cx),
        move |bounds, direction, window, _| {
            let (origin, size) = rect_of(bounds);
            let pieces = stroke_border_pieces(origin, size, corner_radius, line_width);
            paint_pieces(&gradient, &pieces, bounds, direction, window);
        },
    )
    .size_full()
}

fn rect_of(bounds: Bounds<Pixels>) -> (GradientPoint, GradientPoint) {
    (
        (f32::from(bounds.origin.x), f32::from(bounds.origin.y)),
        (f32::from(bounds.size.width), f32::from(bounds.size.height)),
    )
}

/// outline clockwise from the end of the top left arc
/// always the same number of points, so a stroke can pair outer and inner outlines
pub(crate) fn rounded_outline(origin: GradientPoint, size: GradientPoint, corner_radius: f32) -> Vec<GradientPoint> {
    let radius = corner_radius.max(0.0).min(size.0.min(size.1) / 2.0);
    let (left, top) = (origin.0 + radius, origin.1 + radius);
    let (right, bottom) = (origin.0 + size.0 - radius, origin.1 + size.1 - radius);
    let corners = [
        (right, top, -90.0_f32),
        (right, bottom, 0.0_f32),
        (left, bottom, 90.0_f32),
        (left, top, 180.0_f32),
    ];
    let mut outline = Vec::with_capacity(4 * (CORNER_SEGMENTS + 1));
    for (center_x, center_y, start_degrees) in corners {
        for step in 0..=CORNER_SEGMENTS {
            let degrees = start_degrees + 90.0 * step as f32 / CORNER_SEGMENTS as f32;
            let radians = degrees.to_radians();
            outline.push((center_x + radius * radians.cos(), center_y + radius * radians.sin()));
        }
    }
    outline
}

/// ring between the outline and the outline inset by line_width, as convex quads
pub(crate) fn stroke_border_pieces(
    origin: GradientPoint,
    size: GradientPoint,
    corner_radius: f32,
    line_width: f32,
) -> Vec<Vec<GradientPoint>> {
    let width = line_width.max(0.0).min(size.0.min(size.1) / 2.0);
    let outer = rounded_outline(origin, size, corner_radius);
    let inner = rounded_outline(
        (origin.0 + width, origin.1 + width),
        (size.0 - width * 2.0, size.1 - width * 2.0),
        (corner_radius - width).max(0.0),
    );
    (0..outer.len())
        .map(|index| {
            let next = (index + 1) % outer.len();
            vec![outer[index], outer[next], inner[next], inner[index]]
        })
        .collect()
}

/// sutherland hodgman against one half plane, keeps points where side is not negative
pub(crate) fn keep_side(piece: &[GradientPoint], side: impl Fn(GradientPoint) -> f32) -> Vec<GradientPoint> {
    let mut kept = Vec::with_capacity(piece.len() + 1);
    for (index, &current) in piece.iter().enumerate() {
        let next = piece[(index + 1) % piece.len()];
        let (current_side, next_side) = (side(current), side(next));
        if current_side >= 0.0 {
            kept.push(current);
        }
        if (current_side >= 0.0) != (next_side >= 0.0) {
            let t = current_side / (current_side - next_side);
            kept.push((current.0 + (next.0 - current.0) * t, current.1 + (next.1 - current.1) * t));
        }
    }
    kept
}

fn clip_pieces(
    pieces: &[Vec<GradientPoint>],
    axis: &GradientAxis,
    from: Option<f32>,
    to: Option<f32>,
) -> Vec<Vec<GradientPoint>> {
    pieces
        .iter()
        .map(|piece| {
            let piece = match from {
                Some(from) => keep_side(piece, |p| axis.project(p) - from),
                None => piece.clone(),
            };
            match to {
                Some(to) => keep_side(&piece, |p| to - axis.project(p)),
                None => piece,
            }
        })
        .filter(|piece| piece.len() >= 3)
        .collect()
}

fn bounding_rect(pieces: &[Vec<GradientPoint>]) -> Option<GradientPolygon> {
    let mut points = pieces.iter().flatten();
    let &first = points.next()?;
    let (mut min, mut max) = (first, first);
    for &(x, y) in points {
        min = (min.0.min(x), min.1.min(y));
        max = (max.0.max(x), max.1.max(y));
    }
    Some(GradientPolygon::rect(min, (max.0 - min.0, max.1 - min.1)))
}

fn paint_as_one_path(pieces: &[Vec<GradientPoint>], background: Background, window: &mut Window) {
    let mut contours = pieces.iter();
    let Some(first) = contours.next() else {
        return;
    };
    let mut path = Path::new(point(px(first[0].0), px(first[0].1)));
    for &(x, y) in &first[1..] {
        path.line_to(point(px(x), px(y)));
    }
    for contour in contours {
        path.move_to(point(px(contour[0].0), px(contour[0].1)));
        for &(x, y) in &contour[1..] {
            path.line_to(point(px(x), px(y)));
        }
    }
    window.paint_path(path, background);
}

fn paint_pieces(
    gradient: &LinearGradient,
    pieces: &[Vec<GradientPoint>],
    bounds: Bounds<Pixels>,
    direction: LayoutDirection,
    window: &mut Window,
) {
    let Some(first) = gradient.stops.first() else {
        return;
    };
    if gradient.stops.len() == 1 {
        paint_as_one_path(pieces, first.color.into(), window);
        return;
    }
    let (origin, size) = rect_of(bounds);
    let Some(axis) = GradientAxis::new(
        origin,
        size,
        gradient.start_point.resolved(direction),
        gradient.end_point.resolved(direction),
    ) else {
        return;
    };

    let first_location = first.location.clamp(0.0, 1.0);
    let before = clip_pieces(pieces, &axis, None, Some(axis.distance(first_location)));
    paint_as_one_path(&before, first.color.into(), window);

    let mut previous: (Hsla, f32) = (first.color, first_location);
    for stop in &gradient.stops[1..] {
        let location = stop.location.clamp(0.0, 1.0).max(previous.1);
        let (from, to) = (axis.distance(previous.1), axis.distance(location));
        let band = clip_pieces(pieces, &axis, Some(from), Some(to));
        if let Some(rect) = bounding_rect(&band)
            && let Some(background) = band_background(&rect, &axis, (previous.0, from), (stop.color, to))
        {
            paint_as_one_path(&band, background, window);
        }
        previous = (stop.color, location);
    }

    let after = clip_pieces(pieces, &axis, Some(axis.distance(previous.1)), None);
    paint_as_one_path(&after, previous.0.into(), window);
}
