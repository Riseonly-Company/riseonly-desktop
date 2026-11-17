use gpui::{Hsla, Rgba, black, white};

use super::radial_gradient::{RadialGeometry, RadialGradient, farthest_corner};
use crate::swiftui::transition::UnitPoint;

const TOLERANCE: f32 = 1e-3;

fn rgb(color: Hsla) -> Rgba {
    color.to_rgb()
}

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < TOLERANCE
}

#[test]
fn colours_spread_evenly_between_the_radii() {
    let gradient = RadialGradient::new([black(), white(), black()], UnitPoint::CENTER, 10.0, 30.0);
    assert!(close(gradient.stop_radius(0), 10.0));
    assert!(close(gradient.stop_radius(1), 20.0));
    assert!(close(gradient.stop_radius(2), 30.0));
    assert!(close(rgb(gradient.color_at_distance(15.0)).r, 0.5));
    assert!(close(rgb(gradient.color_at_distance(20.0)).r, 1.0));
}

#[test]
fn end_colours_pad_inside_the_start_and_past_the_end() {
    let gradient = RadialGradient::new([black(), white()], UnitPoint::CENTER, 10.0, 20.0);
    assert!(close(rgb(gradient.color_at_distance(0.0)).r, 0.0));
    assert!(close(rgb(gradient.color_at_distance(500.0)).r, 1.0));
}

#[test]
fn equal_radii_split_into_the_two_end_colours() {
    let gradient = RadialGradient::new([black(), white()], UnitPoint::CENTER, 12.0, 12.0);
    assert!(close(rgb(gradient.color_at_distance(11.0)).r, 0.0));
    assert!(close(rgb(gradient.color_at_distance(12.0)).r, 1.0));
    assert!(close(gradient.stop_radius(1), 12.0));
}

#[test]
fn degenerate_gradients_have_a_colour() {
    let empty = RadialGradient::new([], UnitPoint::CENTER, 0.0, 10.0);
    assert_eq!(empty.color_at(0.3), Hsla::transparent_black());
    let single = RadialGradient::new([white()], UnitPoint::CENTER, 0.0, 10.0);
    assert_eq!(single.color_at_distance(4.0), white());
}

#[test]
fn farthest_corner_reaches_the_whole_view() {
    let far = farthest_corner((50.0, 42.0), (0.0, 0.0), (100.0, 100.0));
    assert!(close(far, (50.0_f32 * 50.0 + 58.0 * 58.0).sqrt()));
}

#[test]
fn outer_ring_covers_the_corners_between_sectors() {
    let gradient = RadialGradient::new([black(), white()], UnitPoint::new(0.5, 0.42), 0.0, 75.0);
    let geometry = RadialGeometry::new((0.0, 0.0), (100.0, 100.0), gradient.center, &gradient);
    assert!(geometry.sectors >= 48);
    let covering = geometry.covering(geometry.far);
    let half = (180.0 / geometry.sectors as f32).to_radians();
    assert!(covering * half.cos() >= geometry.far - TOLERANCE);
}
