use gpui::{Hsla, Rgba, black, white};

use super::angular_gradient::{AngularGradient, point_on_circle, sector_count};

const TOLERANCE: f32 = 1e-3;

fn rgb(color: Hsla) -> Rgba {
    color.to_rgb()
}

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < TOLERANCE
}

#[test]
fn colours_spread_evenly_over_the_turn() {
    let gradient = AngularGradient::new([black(), white(), black()]);
    assert!(close(rgb(gradient.color_at(0.0)).r, 0.0));
    assert!(close(rgb(gradient.color_at(0.5)).r, 1.0));
    assert!(close(rgb(gradient.color_at(0.25)).r, 0.5));
    assert!(close(rgb(gradient.color_at(1.0)).r, 0.0));
}

#[test]
fn zero_degrees_points_right_and_grows_clockwise() {
    let (x, y) = point_on_circle((0.0, 0.0), 10.0, 0.0);
    assert!(close(x, 10.0) && close(y, 0.0));
    let (x, y) = point_on_circle((0.0, 0.0), 10.0, 90.0);
    assert!(close(x, 0.0) && close(y, 10.0));
}

#[test]
fn start_angle_rotates_the_colours() {
    let gradient = AngularGradient::new([black(), white()]).angle(90.0);
    assert!(close(rgb(gradient.color_at_angle(90.0)).r, 0.0));
    assert!(close(rgb(gradient.color_at_angle(270.0)).r, 0.5));
    assert!(close(rgb(gradient.color_at_angle(-90.0)).r, 0.5));
}

#[test]
fn degenerate_gradients_have_a_colour() {
    assert_eq!(AngularGradient::new([]).color_at(0.3), Hsla::transparent_black());
    assert_eq!(AngularGradient::new([white()]).color_at(0.7), white());
}

#[test]
fn ring_sectors_stay_bounded() {
    assert_eq!(sector_count(0.0), 48);
    assert_eq!(sector_count(10.0), 48);
    assert!(sector_count(60.0) > 48);
    assert_eq!(sector_count(10_000.0), 720);
}
