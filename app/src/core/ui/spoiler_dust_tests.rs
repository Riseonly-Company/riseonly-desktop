use crate::core::ui::spoiler_dust::emitter_spread;

fn close(left: f32, right: f32) -> bool {
    (left - right).abs() < 1e-5
}

#[test]
fn a_zero_draw_lands_on_the_low_end_of_the_range() {
    assert!(close(emitter_spread(1.4, 0.6, 0.0), 0.8));
    assert!(close(emitter_spread(3.0, 5.0, 0.0), -2.0));
    assert!(close(emitter_spread(1.0, 0.4, 0.0), 0.6));
    assert!(close(emitter_spread(1.0, 0.6, 0.0), 0.4));
}

#[test]
fn a_draw_near_one_reaches_the_high_end_of_the_range() {
    let top = 1.0 - f32::EPSILON;
    assert!(close(emitter_spread(1.4, 0.6, top), 2.0));
    assert!(close(emitter_spread(3.0, 5.0, top), 8.0));
    assert!(close(emitter_spread(1.0, 0.4, top), 1.4));
    assert!(close(emitter_spread(1.0, 0.6, top), 1.6));
}

#[test]
fn a_middle_draw_is_the_value_itself() {
    assert!(close(emitter_spread(1.4, 0.6, 0.5), 1.4));
    assert!(close(emitter_spread(3.0, 5.0, 0.5), 3.0));
}
