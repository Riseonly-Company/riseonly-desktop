use std::time::Duration;

use gpui::{Bounds, point, px, size};

use crate::live_voice_bars_ui::{BREATH_PERIOD, bar_frames, bar_scale, keyframe_value};

#[test]
fn keyframes_hit_their_values_at_the_key_times() {
    assert!((keyframe_value(0.) - 0.38).abs() < 1e-5);
    assert!((keyframe_value(0.28) - 1.0).abs() < 1e-5);
    assert!((keyframe_value(0.52) - 0.55).abs() < 1e-5);
    assert!((keyframe_value(0.76) - 0.9).abs() < 1e-5);
    assert!((keyframe_value(1.) - 0.38).abs() < 1e-5);
}

#[test]
fn later_bars_show_the_model_value_until_their_begin_time() {
    assert_eq!(bar_scale(1, Duration::from_millis(100)), 1.0);
    assert_eq!(bar_scale(2, Duration::from_millis(800)), 1.0);
    assert!((bar_scale(0, Duration::ZERO) - 0.38).abs() < 1e-5);
    assert!((bar_scale(0, BREATH_PERIOD) - 0.38).abs() < 1e-4);
}

#[test]
fn bars_split_the_badge_into_fifths() {
    let frames = bar_frames(Bounds::new(point(px(0.), px(0.)), size(px(11.), px(10.))));
    let width = 11. / 5.;
    assert!((f32::from(frames[0].size.width) - width).abs() < 1e-5);
    assert!((f32::from(frames[0].origin.x) - 0.).abs() < 1e-5);
    assert!((f32::from(frames[2].origin.x) - 4. * width).abs() < 1e-5);
    assert_eq!(f32::from(frames[1].size.height), 10.);
}
