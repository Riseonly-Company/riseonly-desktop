use crate::swiftui::slider::{KNOB_SIZE, SliderScale};

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

#[test]
fn the_value_follows_the_knob_centre() {
    let scale = SliderScale::new(0.0..=1.0, None);
    let width = 228.0;
    assert!(close(scale.value_at(KNOB_SIZE / 2.0, width), 0.0));
    assert!(close(scale.value_at(width - KNOB_SIZE / 2.0, width), 1.0));
    assert!(close(scale.value_at(width / 2.0, width), 0.5));
}

#[test]
fn positions_past_the_ends_clamp_into_the_range() {
    let scale = SliderScale::new(-10.0..=10.0, None);
    assert!(close(scale.value_at(-50.0, 128.0), -10.0));
    assert!(close(scale.value_at(500.0, 128.0), 10.0));
    assert!(close(scale.clamp(42.0), 10.0));
    assert!(close(scale.fraction(-42.0), 0.0));
}

#[test]
fn a_step_rounds_to_the_nearest_stop_from_the_lower_bound() {
    let scale = SliderScale::new(1.0..=10.0, Some(2.0));
    assert!(close(scale.quantize(2.4), 3.0));
    assert!(close(scale.quantize(1.9), 1.0));
    assert!(close(scale.quantize(10.0), 10.0), "a top the step does not reach is clamped");
}

#[test]
fn arrow_keys_move_by_step_or_a_hundredth() {
    let stepped = SliderScale::new(0.0..=100.0, Some(5.0));
    assert_eq!(stepped.stepped(50.0, 1.0), Some(55.0));
    assert_eq!(stepped.stepped(100.0, 1.0), None);
    let continuous = SliderScale::new(0.0..=2.0, None);
    assert!(close(continuous.keyboard_step(), 0.02));
    assert_eq!(continuous.stepped(0.0, -1.0), None);
}

#[test]
fn a_slider_too_narrow_for_the_knob_shows_the_lower_bound() {
    let scale = SliderScale::new(3.0..=7.0, None);
    assert!(close(scale.value_at(10.0, KNOB_SIZE), 3.0));
}
