use gpui::{point, px, size};

use crate::liquid_glass_button::{LiquidGlassPressMetrics, LiquidPressShape, LiquidPressState};

#[test]
fn press_metrics_are_the_ios_values() {
    assert_eq!(LiquidGlassPressMetrics::PRESS_SCALE, 1.06);
    assert_eq!(LiquidGlassPressMetrics::MAX_OFFSET, 6.);
    assert_eq!(LiquidGlassPressMetrics::RELEASE_SLOP, 18.);
}

#[test]
fn capsule_corners_belong_to_what_is_behind() {
    let frame = size(px(100.), px(40.));
    assert!(LiquidPressShape::Capsule.contains(point(px(50.), px(20.)), frame));
    assert!(!LiquidPressShape::Capsule.contains(point(px(1.), px(1.)), frame));
    assert!(LiquidPressShape::RoundedRect(4.).contains(point(px(2.), px(2.)), frame));
    assert!(!LiquidPressShape::Circle.contains(point(px(1.), px(1.)), size(px(40.), px(40.))));
    assert!(!LiquidPressShape::Circle.contains(point(px(-1.), px(20.)), size(px(40.), px(40.))));
}

#[test]
fn a_fresh_press_is_at_rest() {
    let state = LiquidPressState::new();
    assert!(!state.is_pressed());
    assert_eq!(state.pressed_progress(std::time::Duration::ZERO), 0.);
}
