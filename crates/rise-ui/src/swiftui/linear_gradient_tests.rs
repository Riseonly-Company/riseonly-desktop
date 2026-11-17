use gpui::{Hsla, black, white};

use crate::swiftui::linear_gradient::{
    GradientAxis, GradientPolygon, GradientStop, LinearGradient, gpui_gradient_for,
    gpui_gradient_parameter,
};
use crate::swiftui::transition::UnitPoint;

const TOLERANCE: f32 = 1e-3;

/// color position gpui shades at p for a band, as its shader does after the stops
fn shaded_position(
    origin: (f32, f32),
    size: (f32, f32),
    vector: (f32, f32),
    from: (f32, f32),
    to: (f32, f32),
    p: (f32, f32),
) -> f32 {
    let (angle, start, end) =
        gpui_gradient_for(origin, size, vector, from, to).expect("band has a direction");
    (gpui_gradient_parameter(angle, origin, size, p) - start) / (end - start)
}

#[test]
fn vertical_band_runs_from_its_top_stop_to_its_bottom_stop() {
    let origin = (10.0, 20.0);
    let size = (100.0, 200.0);
    let axis = GradientAxis::new(origin, size, UnitPoint::TOP, UnitPoint::BOTTOM)
        .expect("axis has length");
    let band = GradientPolygon::rect(origin, size).clip(
        &axis,
        Some(axis.distance(0.25)),
        Some(axis.distance(0.5)),
    );
    let (band_origin, band_size) = band.bounding_box();
    assert!((band_origin.1 - 70.0).abs() < TOLERANCE);
    assert!((band_size.1 - 50.0).abs() < TOLERANCE);

    let from = axis.point_at(axis.distance(0.25));
    let to = axis.point_at(axis.distance(0.5));
    let vector = (0.0, 200.0);
    let top = shaded_position(band_origin, band_size, vector, from, to, (40.0, 70.0));
    let middle = shaded_position(band_origin, band_size, vector, from, to, (90.0, 95.0));
    let bottom = shaded_position(band_origin, band_size, vector, from, to, (15.0, 120.0));
    assert!(top.abs() < TOLERANCE, "top {top}");
    assert!((middle - 0.5).abs() < TOLERANCE, "middle {middle}");
    assert!((bottom - 1.0).abs() < TOLERANCE, "bottom {bottom}");
}

#[test]
fn diagonal_gradient_shades_along_the_corner_to_corner_line_of_a_wide_box() {
    let origin = (0.0, 0.0);
    let size = (300.0, 100.0);
    let vector = (300.0, 100.0);
    let length_squared = 300.0 * 300.0 + 100.0 * 100.0;
    for p in [(0.0, 0.0), (300.0, 100.0), (0.0, 100.0), (300.0, 0.0), (120.0, 40.0)] {
        let expected = (p.0 * vector.0 + p.1 * vector.1) / length_squared;
        let shaded = shaded_position(origin, size, vector, (0.0, 0.0), (300.0, 100.0), p);
        assert!((shaded - expected).abs() < TOLERANCE, "{p:?}: {shaded} != {expected}");
    }
}

#[test]
fn diagonal_band_of_a_tall_box_keeps_iso_lines_perpendicular_to_the_axis() {
    let origin = (0.0, 0.0);
    let size = (80.0, 400.0);
    let axis = GradientAxis::new(origin, size, UnitPoint::TOP_LEADING, UnitPoint::BOTTOM_TRAILING)
        .expect("axis has length");
    let from_distance = axis.distance(0.3);
    let to_distance = axis.distance(0.7);
    let band =
        GradientPolygon::rect(origin, size).clip(&axis, Some(from_distance), Some(to_distance));
    assert!(band.len >= 3);
    let (band_origin, band_size) = band.bounding_box();
    let from = axis.point_at(from_distance);
    let to = axis.point_at(to_distance);
    for &corner in &band.points[..band.len] {
        let expected = (axis.project(corner) - from_distance) / (to_distance - from_distance);
        let shaded = shaded_position(band_origin, band_size, (80.0, 400.0), from, to, corner);
        assert!((shaded - expected).abs() < TOLERANCE, "{corner:?}: {shaded} != {expected}");
        assert!((-TOLERANCE..=1.0 + TOLERANCE).contains(&expected));
    }
}

#[test]
fn bands_and_padding_cover_the_rect_without_overlap() {
    let origin = (0.0, 0.0);
    let size = (120.0, 90.0);
    let axis = GradientAxis::new(origin, size, UnitPoint::LEADING, UnitPoint::TRAILING)
        .expect("axis has length");
    let rect = GradientPolygon::rect(origin, size);
    let cuts = [0.0, 0.14, 0.48, 1.0].map(|location| axis.distance(location));
    let mut covered = 0.0;
    let mut previous = None;
    for cut in cuts {
        let (_, part) = rect.clip(&axis, previous, Some(cut)).bounding_box();
        covered += part.0.max(0.0);
        previous = Some(cut);
    }
    let (_, rest) = rect.clip(&axis, previous, None).bounding_box();
    covered += rest.0.max(0.0);
    assert!((covered - 120.0).abs() < TOLERANCE, "covered {covered}");
}

#[test]
fn start_equal_to_end_has_no_axis() {
    let axis = GradientAxis::new((0.0, 0.0), (50.0, 50.0), UnitPoint::CENTER, UnitPoint::CENTER);
    assert!(axis.is_none());
}

#[test]
fn colors_spread_evenly_like_swiftui() {
    let colors: [Hsla; 3] = [black(), white(), black()];
    let gradient = LinearGradient::colors(colors, UnitPoint::TOP, UnitPoint::BOTTOM);
    let locations: Vec<f32> = gradient.stops.iter().map(|stop| stop.location).collect();
    assert_eq!(locations, vec![0.0, 0.5, 1.0]);
    assert_eq!(gradient.stops[1], GradientStop::new(white(), 0.5));

    let single = LinearGradient::colors([white()], UnitPoint::TOP, UnitPoint::BOTTOM);
    assert_eq!(single.stops, vec![GradientStop::new(white(), 0.0)]);
}
