use gpui::{Bounds, Pixels, Point, Size, point, px, size};

use crate::core::shared_element_transition::kernel::shared_element_motion::SharedElementMotion;
use crate::core::shared_element_transition::kernel::shared_element_types::{
    SharedElementGesturePolicy, SharedElementGesturePolicyAxis, SharedElementGesturePolicyDecision,
    SharedElementGesturePolicyInput,
};

fn viewport() -> Size<Pixels> {
    size(px(390.0), px(844.0))
}

fn source() -> Bounds<Pixels> {
    Bounds::new(point(px(0.0), px(180.0)), size(px(390.0), px(488.0)))
}

fn vector(dx: f32, dy: f32) -> Point<Pixels> {
    point(px(dx), px(dy))
}

fn input_with(dx: f32, dy: f32, vx: f32, vy: f32, layout_sign: f32) -> SharedElementGesturePolicyInput {
    SharedElementGesturePolicyInput {
        translation: vector(dx, dy),
        velocity: vector(vx, vy),
        touch_start: point(px(120.0), px(420.0)),
        source: source(),
        viewport: viewport(),
        layout_sign,
        axis: SharedElementGesturePolicyAxis::default(),
    }
}

fn input(dx: f32, dy: f32, vx: f32, vy: f32) -> SharedElementGesturePolicyInput {
    input_with(dx, dy, vx, vy, 1.0)
}

fn decision(input: SharedElementGesturePolicyInput) -> SharedElementGesturePolicyDecision {
    SharedElementGesturePolicy::decision(&input)
}

fn viewport_width() -> f32 {
    f32::from(viewport().width)
}

#[test]
fn backward_velocity_cancels_even_past_the_distance_threshold() {
    let past = viewport_width() * SharedElementMotion::COMMIT_DISTANCE_FRACTION + 40.0;
    assert_eq!(decision(input(past, 0.0, -201.0, 0.0)), SharedElementGesturePolicyDecision::Cancel);
    assert_eq!(decision(input(past, 0.0, -200.0, 0.0)), SharedElementGesturePolicyDecision::Finish);
}

#[test]
fn distance_past_the_commit_fraction_finishes() {
    assert_eq!(decision(input(120.0, 0.0, 0.0, 0.0)), SharedElementGesturePolicyDecision::Finish);
    assert_eq!(decision(input(100.0, 0.0, 0.0, 0.0)), SharedElementGesturePolicyDecision::Cancel);
}

#[test]
fn horizontal_flick_finishes() {
    assert_eq!(decision(input(20.0, 0.0, 900.0, 100.0)), SharedElementGesturePolicyDecision::Finish);
    assert_eq!(decision(input(20.0, 0.0, 900.0, 1000.0)), SharedElementGesturePolicyDecision::Cancel);
    assert_eq!(decision(input(20.0, 0.0, 800.0, 0.0)), SharedElementGesturePolicyDecision::Cancel);
}

#[test]
fn long_drag_in_any_direction_finishes_unless_moving_back() {
    assert_eq!(decision(input(10.0, 200.0, 0.0, 0.0)), SharedElementGesturePolicyDecision::Finish);
    assert_eq!(decision(input(10.0, 200.0, -250.0, 0.0)), SharedElementGesturePolicyDecision::Cancel);
    assert_eq!(decision(input(10.0, 120.0, 0.0, 0.0)), SharedElementGesturePolicyDecision::Cancel);
}

#[test]
fn resting_sample_is_the_source() {
    let frame = SharedElementGesturePolicy::sample(&input(0.0, 0.0, 0.0, 0.0));
    assert_eq!(frame.scale, 1.0);
    assert_eq!(frame.clip, source());
    assert_eq!(frame.chrome_alpha, 1.0);
    assert_eq!(frame.progress, 0.0);
}

#[test]
fn rubber_band_is_monotone() {
    let mut previous_scale: f32 = 1.0;
    let mut previous_width = f32::from(source().size.width);
    let mut previous_alpha: f32 = 1.0;
    for step in 0..=200 {
        let step = step as f32;
        let frame = SharedElementGesturePolicy::sample(&input(step * 12.0, step * 5.0, 0.0, 0.0));
        let width = f32::from(frame.clip.size.width);
        assert!(frame.scale <= previous_scale);
        assert!(width <= previous_width + 1e-3);
        assert!(frame.chrome_alpha <= previous_alpha);
        previous_scale = frame.scale;
        previous_width = width;
        previous_alpha = frame.chrome_alpha;
    }
}

#[test]
fn scale_never_drops_below_the_floor() {
    for distance in [0.0, 100.0, 1_000.0, 100_000.0, 1e12] {
        let frame = SharedElementGesturePolicy::sample(&input(distance, distance, 0.0, 0.0));
        assert!(frame.scale >= SharedElementMotion::SCALE_FLOOR);
    }
}

#[test]
fn grabbed_point_stays_under_the_finger() {
    let sample = input(140.0, 60.0, 0.0, 0.0);
    let frame = SharedElementGesturePolicy::sample(&sample);
    let source = source();
    let touch_x = f32::from(sample.touch_start.x);
    let touch_y = f32::from(sample.touch_start.y);
    let fraction_x = (touch_x - f32::from(source.left())) / f32::from(source.size.width);
    let fraction_y = (touch_y - f32::from(source.top())) / f32::from(source.size.height);
    let grabbed_x = f32::from(frame.clip.left()) + fraction_x * f32::from(frame.clip.size.width);
    let grabbed_y = f32::from(frame.clip.top()) + fraction_y * f32::from(frame.clip.size.height);
    assert!((grabbed_x - (touch_x + 140.0)).abs() < 1e-3);
    assert!((grabbed_y - (touch_y + 60.0)).abs() < 1e-3);
}

#[test]
fn chrome_fades_out_by_the_fade_distance() {
    let fade_distance = viewport_width() * SharedElementMotion::CHROME_FADE_DISTANCE_FRACTION;
    assert_eq!(SharedElementGesturePolicy::sample(&input(fade_distance, 0.0, 0.0, 0.0)).chrome_alpha, 0.0);
    let halfway = SharedElementGesturePolicy::sample(&input(fade_distance / 2.0, 0.0, 0.0, 0.0)).chrome_alpha;
    assert!(halfway > 0.0 && halfway < 1.0);
}

#[test]
fn right_to_left_sign_mirrors_progress_and_decision() {
    let rtl = input_with(-60.0, 0.0, -900.0, 0.0, -1.0);
    let ltr = input_with(-60.0, 0.0, -900.0, 0.0, 1.0);

    assert!((SharedElementGesturePolicy::sample(&rtl).progress - 60.0 / viewport_width()).abs() < 1e-6);
    assert_eq!(SharedElementGesturePolicy::sample(&ltr).progress, 0.0);
    assert_eq!(decision(rtl), SharedElementGesturePolicyDecision::Finish);
    assert_eq!(decision(ltr), SharedElementGesturePolicyDecision::Cancel);
}

#[test]
fn progress_clamps_to_one() {
    assert_eq!(SharedElementGesturePolicy::sample(&input(2_000.0, 0.0, 0.0, 0.0)).progress, 1.0);
}

#[test]
fn relative_spring_velocity_projects_and_clamps() {
    let from = point(px(0.0), px(0.0));
    let to = point(px(0.0), px(100.0));
    assert!((SharedElementGesturePolicy::relative_spring_velocity(vector(0.0, 500.0), from, to) - 5.0).abs() < 1e-6);
    assert_eq!(
        SharedElementGesturePolicy::relative_spring_velocity(vector(0.0, 50_000.0), from, to),
        SharedElementMotion::MAX_RELATIVE_VELOCITY
    );
    assert_eq!(SharedElementGesturePolicy::relative_spring_velocity(vector(0.0, -500.0), from, to), 0.0);
    assert_eq!(SharedElementGesturePolicy::relative_spring_velocity(vector(900.0, 0.0), from, to), 0.0);
    assert_eq!(
        SharedElementGesturePolicy::relative_spring_velocity(vector(0.0, 500.0), from, point(px(0.5), px(0.5))),
        0.0
    );
}
