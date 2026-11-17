use std::time::Duration;

use gpui::{hsla, px, Hsla, Pixels, Point};

use crate::swiftui::animated::{Animated, Lerp};
use crate::swiftui::motion::{ease_in_out, interactive_spring, linear, spring, Spring};

fn at(seconds: f64) -> Duration {
    Duration::from_secs_f64(seconds)
}

fn assert_close(actual: f32, expected: f32, tolerance: f32, what: &str) {
    assert!(
        (actual - expected).abs() <= tolerance,
        "{what}: expected {expected}, got {actual}"
    );
}

#[test]
fn a_new_value_is_at_rest() {
    let value = Animated::new(4.0_f32);
    assert_eq!(value.value(at(1.0)), 4.0);
    assert!(!value.is_running(at(1.0)));
}

#[test]
fn a_curve_runs_from_the_visible_value_to_the_target() {
    let mut value = Animated::new(0.0_f32);
    value.set(100.0, ease_in_out(1.0), at(10.0), false);

    assert!(value.is_running(at(10.0)));
    assert_close(value.value(at(10.0)), 0.0, 1e-4, "start");
    assert_close(value.value(at(10.25)), 12.9162, 1e-2, "quarter");
    assert_close(value.value(at(10.5)), 50.0, 1e-2, "middle");
    assert_close(value.value(at(11.0)), 100.0, 1e-4, "end");
    assert!(!value.is_running(at(11.0)));
    assert_eq!(*value.target(), 100.0);
}

#[test]
fn reduce_motion_lands_on_the_target_at_once() {
    let mut value = Animated::new(0.0_f32);
    value.set(1.0, spring(0.34, 0.84), at(2.0), true);
    assert_eq!(value.value(at(2.0)), 1.0);
    assert!(!value.is_running(at(2.0)));
}

#[test]
fn no_spec_lands_on_the_target_at_once() {
    let mut value = Animated::new(px(0.0));
    value.set(px(20.0), None, at(2.0), false);
    assert_eq!(value.value(at(2.0)), px(20.0));
    assert!(!value.is_running(at(2.0)));
}

#[test]
fn setting_the_same_target_does_not_restart_the_motion() {
    let mut value = Animated::new(0.0_f32);
    value.set(100.0, linear(1.0), at(0.0), false);
    let before = value.value(at(0.5));
    value.set(100.0, linear(1.0), at(0.5), false);
    assert_close(value.value(at(0.5)), before, 1e-4, "same target");
    assert_close(value.value(at(0.75)), 75.0, 1e-3, "still the first run");
}

#[test]
fn a_spring_overshoots_its_target_like_swiftui() {
    let mut value = Animated::new(0.0_f32);
    value.set(1.0, spring(0.34, 0.84), at(0.0), false);
    assert_close(value.value(at(0.313314)), 1.0077224, 1e-5, "overshoot peak");
}

#[test]
fn a_spring_stops_requesting_frames_once_settled() {
    let mut value = Animated::new(0.0_f32);
    value.set(1.0, spring(0.34, 0.84), at(0.0), false);
    let settle = Spring::new(0.34, 0.84).settling_duration().as_secs_f64();

    assert!(value.is_running(at(settle - 0.01)));
    assert!(!value.is_running(at(settle + 0.01)));
    assert_eq!(value.value(at(settle + 0.01)), 1.0);
}

#[test]
fn interrupting_a_spring_keeps_its_velocity_and_position() {
    let mut value = Animated::new(0.0_f32);
    value.set(100.0, spring(0.34, 0.84), at(0.0), false);
    let position = value.value(at(0.1));
    let velocity = value.velocity(at(0.1))[0];
    assert!(velocity > 0.0);

    value.set(200.0, spring(0.34, 0.84), at(0.1), false);
    assert_close(value.value(at(0.1)), position, 1e-3, "position at retarget");
    assert_close(value.velocity(at(0.1))[0], velocity, 1e-2, "velocity at retarget");
    assert!(value.value(at(0.12)) > position);
}

#[test]
fn interrupting_a_curve_starts_from_the_visible_value_at_rest() {
    let mut value = Animated::new(0.0_f32);
    value.set(100.0, ease_in_out(1.0), at(0.0), false);
    let position = value.value(at(0.5));

    value.set(0.0, ease_in_out(1.0), at(0.5), false);
    assert_close(value.value(at(0.5)), position, 1e-3, "position at retarget");
    assert!(value.velocity(at(0.5))[0].abs() < 1.0);
    assert_close(value.value(at(1.5)), 0.0, 1e-4, "end of the new run");
}

#[test]
fn a_spring_replacing_a_spring_with_blend_stays_continuous_and_settles() {
    let mut value = Animated::new(0.0_f32);
    value.set(100.0, spring(0.5, 0.8), at(0.0), false);
    let position = value.value(at(0.1));

    value.set(50.0, interactive_spring(0.15, 0.86, 0.25), at(0.1), false);
    assert_close(value.value(at(0.1)), position, 1e-2, "position at retarget");

    let mut previous = value.value(at(0.1));
    for ms in 101..=360 {
        let current = value.value(at(f64::from(ms) / 1000.0));
        assert!((current - previous).abs() < 5.0, "jump at {ms} ms: {previous} -> {current}");
        previous = current;
    }
    assert!(!value.is_running(at(5.0)));
    assert_eq!(value.value(at(5.0)), 50.0);
}

#[test]
fn a_delay_holds_the_previous_value() {
    let mut value = Animated::new(0.0_f32);
    value.set(10.0, linear(1.0).delay(0.5), at(0.0), false);
    assert_eq!(value.value(at(0.25)), 0.0);
    assert!(value.is_running(at(0.25)));
    assert_close(value.value(at(1.0)), 5.0, 1e-3, "half way after the delay");
    assert!(!value.is_running(at(1.5)));
}

#[test]
fn a_counted_repeat_reverses_and_ends_on_the_target() {
    let mut value = Animated::new(0.0_f32);
    value.set(10.0, linear(1.0).repeat_count(3, true), at(0.0), false);
    assert_close(value.value(at(0.5)), 5.0, 1e-3, "first pass");
    assert_close(value.value(at(1.25)), 7.5, 1e-3, "reversed pass");
    assert!(value.is_running(at(2.9)));
    assert!(!value.is_running(at(3.0)));
    assert_eq!(value.value(at(3.0)), 10.0);
}

#[test]
fn a_forever_repeat_keeps_running() {
    let mut value = Animated::new(0.0_f32);
    value.set(1.0, linear(1.15).repeat_forever(false), at(0.0), false);
    assert!(value.is_running(at(1000.0)));
    assert!(value.value(at(1.15 * 10.0 + 0.575)) > 0.4);
}

#[test]
fn colours_interpolate_through_rgb_channels_not_hue() {
    let red = hsla(0.0, 1.0, 0.5, 1.0);
    let blue = hsla(2.0 / 3.0, 1.0, 0.5, 1.0);
    let middle = red.lerp(&blue, 0.5).components();
    assert_close(middle[0], 0.5, 1e-3, "red channel");
    assert_close(middle[1], 0.0, 1e-3, "green channel");
    assert_close(middle[2], 0.5, 1e-3, "blue channel");
    assert_close(middle[3], 1.0, 1e-3, "alpha");
}

#[test]
fn an_overshooting_colour_is_clamped_to_valid_channels() {
    let black = hsla(0.0, 0.0, 0.0, 1.0);
    let white = hsla(0.0, 0.0, 1.0, 1.0);
    let beyond: Hsla = black.lerp(&white, 1.2);
    for channel in beyond.components() {
        assert!((0.0..=1.0).contains(&channel));
    }
}

#[test]
fn points_move_on_every_axis() {
    let mut value = Animated::new(Point { x: px(0.0), y: px(0.0) });
    value.set(Point { x: px(10.0), y: px(-20.0) }, linear(1.0), at(0.0), false);
    let middle = value.value(at(0.5));
    assert_close(middle.x.as_f32(), 5.0, 1e-3, "x");
    assert_close(middle.y.as_f32(), -10.0, 1e-3, "y");
}

#[test]
fn pixels_settle_at_a_subpixel_threshold() {
    assert!(<Pixels as Lerp>::resting_threshold() <= 0.01);
    assert!(<f32 as Lerp>::resting_threshold() <= 0.001);
}
