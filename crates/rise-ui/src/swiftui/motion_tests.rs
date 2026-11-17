use std::f64::consts::PI;
use std::time::Duration;

use crate::swiftui::motion::{
    duration_from_seconds, ease_in_out, ease_out, linear, snappy, spring, spring_bounce, timing_curve, CubicBezier,
    MotionSpec, Repeat, Spring, Timing, DEFAULT_ANIMATION_DURATION,
};

fn assert_close(actual: f64, expected: f64, tolerance: f64, what: &str) {
    assert!(
        (actual - expected).abs() <= tolerance,
        "{what}: expected {expected}, got {actual}"
    );
}

#[test]
fn cubic_bezier_matches_ca_media_timing_function_at_quarter_points() {
    let cases = [
        ("easeInEaseOut", CubicBezier::EASE_IN_OUT, [0.129162, 0.5, 0.870838]),
        ("easeIn", CubicBezier::EASE_IN, [0.093465, 0.315357, 0.621862]),
        ("easeOut", CubicBezier::EASE_OUT, [0.378138, 0.684643, 0.906535]),
    ];
    for (name, curve, expected) in cases {
        for (x, y) in [0.25, 0.5, 0.75].into_iter().zip(expected) {
            assert_close(curve.progress(x), y, 1e-5, &format!("{name} at {x}"));
        }
    }
}

#[test]
fn linear_curve_is_the_identity() {
    for step in 0..=20 {
        let x = f64::from(step) / 20.0;
        assert_close(CubicBezier::LINEAR.progress(x), x, 1e-6, "linear");
    }
}

#[test]
fn curves_start_at_zero_end_at_one_and_clamp_time() {
    for curve in [CubicBezier::EASE_IN_OUT, CubicBezier::EASE_IN, CubicBezier::EASE_OUT] {
        assert_eq!(curve.progress(0.0), 0.0);
        assert_eq!(curve.progress(1.0), 1.0);
        assert_eq!(curve.progress(-3.0), 0.0);
        assert_eq!(curve.progress(4.0), 1.0);
    }
}

#[test]
fn ease_in_out_is_flat_at_both_ends_and_steepest_in_the_middle() {
    let curve = CubicBezier::EASE_IN_OUT;
    assert!(curve.slope(0.0) < 0.05);
    assert!(curve.slope(1.0) < 0.05);
    assert!(curve.slope(0.5) > curve.slope(0.25));
}

#[test]
fn gpui_easing_stays_in_the_unit_range_for_an_overshooting_curve() {
    let MotionSpec {
        timing: Timing::Curve { curve, .. },
        ..
    } = timing_curve(0.3, -0.6, 0.7, 1.6, 0.3)
    else {
        panic!("timing_curve must build a curve");
    };
    assert!(curve.progress(0.1) < 0.0, "the custom curve is expected to undershoot");
    let easing = curve.easing();
    for step in 0..=100 {
        let value = easing(step as f32 / 100.0);
        assert!((0.0..=1.0).contains(&value), "with_easing would assert on {value}");
    }
}

#[test]
fn spring_coefficients_follow_response_and_damping_fraction() {
    let spring = Spring::new(0.34, 0.84);
    let omega = 2.0 * PI / 0.34;
    assert_close(spring.angular_frequency(), omega, 1e-9, "omega");
    assert_close(spring.stiffness(), omega * omega, 1e-6, "stiffness");
    assert_close(spring.damping(), 4.0 * PI * 0.84 / 0.34, 1e-9, "damping");
    assert_eq!(Spring::MASS, 1.0);
}

#[test]
fn spring_034_084_overshoots_by_the_damped_oscillator_amount() {
    let spring = Spring::new(0.34, 0.84);
    let (peak, velocity) = spring.state(1.0, 0.0, 0.313314);
    assert_close(-peak, 0.0077224, 1e-6, "overshoot");
    assert_close(velocity, 0.0, 1e-4, "velocity at the peak");

    let deepest = (0..3000)
        .map(|ms| spring.state(1.0, 0.0, f64::from(ms) / 1000.0).0)
        .fold(f64::INFINITY, f64::min);
    assert_close(-deepest, 0.0077224, 1e-5, "deepest swing");
}

#[test]
fn spring_034_084_settles_after_its_last_excursion_past_epsilon() {
    let spring = Spring::new(0.34, 0.84);
    let settle = spring.settling_duration().as_secs_f64();
    assert_close(settle, 0.47219, 1e-3, "settle time");

    for step in 1..3000 {
        let t = settle + f64::from(step) / 1000.0;
        assert!(spring.state(1.0, 0.0, t).0.abs() < 0.001, "swung back out at {t}");
    }
    assert!(spring.state(1.0, 0.0, settle - 0.002).0.abs() >= 0.001);
}

#[test]
fn critically_damped_and_overdamped_springs_never_overshoot() {
    for damping in [1.0, 1.5] {
        let spring = Spring::new(0.5, damping);
        for ms in 0..3000 {
            let x = spring.state(1.0, 0.0, f64::from(ms) / 1000.0).0;
            assert!(x >= -1e-9, "damping {damping} overshot at {ms} ms: {x}");
        }
        assert!(spring.settling_duration() > Duration::ZERO);
    }
}

#[test]
fn spring_state_velocity_is_the_derivative_of_displacement() {
    for damping in [0.6, 1.0, 1.4] {
        let spring = Spring::new(0.3, damping);
        for t in [0.02, 0.1, 0.25] {
            let h = 1e-6;
            let numeric = (spring.state(1.0, 2.0, t + h).0 - spring.state(1.0, 2.0, t - h).0) / (2.0 * h);
            assert_close(spring.state(1.0, 2.0, t).1, numeric, 1e-3, "velocity");
        }
    }
}

#[test]
fn a_spring_at_rest_on_its_target_is_already_settled() {
    assert_eq!(Spring::new(0.34, 0.84).settling_time(0.0, 0.0, 0.001), 0.0);
}

#[test]
fn bounce_maps_to_damping_fraction_as_in_swiftui() {
    let quick = snappy(0.22).as_spring().expect("snappy is a spring");
    assert_close(quick.response, 0.22, 1e-12, "snappy response");
    assert_close(quick.damping_fraction, 0.85, 1e-12, "snappy damping");
    assert_eq!(spring_bounce(0.4, 0.0).as_spring(), Some(Spring::new(0.4, 1.0)));
    assert_eq!(spring_bounce(0.4, -0.5).as_spring(), Some(Spring::new(0.4, 2.0)));
    assert_eq!(spring(0.34, 0.86).as_spring(), Some(Spring::new(0.34, 0.86)));
    assert!(!ease_out(0.2).is_spring());
}

#[test]
fn curve_constructors_carry_the_reference_duration() {
    assert_eq!(
        ease_in_out(0.18).timing,
        Timing::Curve {
            curve: CubicBezier::EASE_IN_OUT,
            duration: Duration::from_millis(180)
        }
    );
    assert_eq!(
        MotionSpec::EASE_OUT.timing,
        Timing::Curve {
            curve: CubicBezier::EASE_OUT,
            duration: DEFAULT_ANIMATION_DURATION
        }
    );
    assert_eq!(DEFAULT_ANIMATION_DURATION, Duration::from_millis(350));
}

#[test]
fn delay_and_repeat_modifiers_compose() {
    let spec = linear(1.15).delay(0.02).repeat_forever(false);
    assert_eq!(spec.delay, Duration::from_millis(20));
    assert_eq!(spec.repeat, Repeat::Forever { autoreverses: false });

    let counted = ease_out(0.3).repeat_count(3, true);
    assert_eq!(
        counted.repeat,
        Repeat::Count {
            count: 3,
            autoreverses: true
        }
    );
}

#[test]
fn seconds_outside_the_valid_range_become_zero_instead_of_panicking() {
    assert_eq!(duration_from_seconds(-1.0), Duration::ZERO);
    assert_eq!(duration_from_seconds(f64::NAN), Duration::ZERO);
    assert_eq!(duration_from_seconds(f64::INFINITY), Duration::ZERO);
    assert_eq!(duration_from_seconds(0.25), Duration::from_millis(250));
}
