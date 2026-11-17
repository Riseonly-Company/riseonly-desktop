use std::time::Duration;

use crate::swiftui::progress_view::{
    ControlSize, LARGE_SIDE, LINEAR_HEIGHT, MEDIUM_SIDE, MIN_SPOKE_OPACITY, ROTATION_PERIOD,
    SPOKE_COUNT, head_spoke, linear_fraction, spoke_angle, spoke_opacity, step_duration,
};

#[test]
fn the_bright_spoke_steps_once_per_eighth_of_a_turn() {
    assert_eq!(step_duration(), Duration::from_millis(125));
    assert_eq!(head_spoke(Duration::ZERO), 0);
    assert_eq!(head_spoke(Duration::from_millis(124)), 0);
    assert_eq!(head_spoke(Duration::from_millis(125)), 1);
    assert_eq!(head_spoke(ROTATION_PERIOD), 0, "a full turn comes back to the first spoke");
    assert_eq!(head_spoke(ROTATION_PERIOD * 3 + Duration::from_millis(900)), 7);
}

#[test]
fn spokes_fade_behind_the_head() {
    assert_eq!(spoke_opacity(3, 3), 1.0);
    assert!((spoke_opacity(4, 3) - MIN_SPOKE_OPACITY).abs() < 1e-6, "the spoke just ahead is the faintest");
    let mut previous = 1.0;
    for behind in 1..SPOKE_COUNT {
        let opacity = spoke_opacity((3 + SPOKE_COUNT - behind) % SPOKE_COUNT, 3);
        assert!(opacity < previous);
        previous = opacity;
    }
}

#[test]
fn spokes_go_clockwise_from_twelve() {
    assert_eq!(spoke_angle(0), 0.0);
    assert!((spoke_angle(2) - std::f32::consts::FRAC_PI_2).abs() < 1e-6);
}

#[test]
fn only_large_changes_the_size_on_ios() {
    assert_eq!(ControlSize::Mini.progress_side(), MEDIUM_SIDE);
    assert_eq!(ControlSize::Small.progress_side(), MEDIUM_SIDE);
    assert_eq!(ControlSize::Regular.progress_side(), MEDIUM_SIDE);
    assert_eq!(ControlSize::Large.progress_side(), LARGE_SIDE);
}

#[test]
fn linear_value_is_pinned_into_zero_one() {
    assert_eq!(LINEAR_HEIGHT, 4.0);
    assert_eq!(linear_fraction(-0.5), 0.0);
    assert_eq!(linear_fraction(0.25), 0.25);
    assert_eq!(linear_fraction(1.0), 1.0);
    assert_eq!(linear_fraction(3.0), 1.0, "an overshoot draws a full bar");
    assert_eq!(linear_fraction(f64::NAN), 0.0, "NaN draws an empty bar");
}
