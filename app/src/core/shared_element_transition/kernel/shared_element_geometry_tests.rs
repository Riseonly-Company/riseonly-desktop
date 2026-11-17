use gpui::{Bounds, Pixels, point, px, size};

use crate::core::shared_element_transition::kernel::shared_element_motion::SharedElementMotion;
use crate::core::shared_element_transition::kernel::shared_element_types::{
    SharedElementCoverSchedule, SharedElementCubicTiming, SharedElementGeometry,
};

const PIXEL_TOLERANCE: f32 = 1e-3;
const UNIT_TOLERANCE: f32 = 1e-5;

fn samples() -> Vec<f32> {
    (0..=20).map(|step| step as f32 / 20.0).collect()
}

fn is_close(a: f32, b: f32, tolerance: f32) -> bool {
    (a - b).abs() <= tolerance
}

fn rect(x: f32, y: f32, width: f32, height: f32) -> Bounds<Pixels> {
    Bounds::new(point(px(x), px(y)), size(px(width), px(height)))
}

fn min_x(rect: Bounds<Pixels>) -> f32 {
    f32::from(rect.left())
}

fn max_x(rect: Bounds<Pixels>) -> f32 {
    f32::from(rect.right())
}

fn min_y(rect: Bounds<Pixels>) -> f32 {
    f32::from(rect.top())
}

fn max_y(rect: Bounds<Pixels>) -> f32 {
    f32::from(rect.bottom())
}

fn width(rect: Bounds<Pixels>) -> f32 {
    f32::from(rect.size.width)
}

fn height(rect: Bounds<Pixels>) -> f32 {
    f32::from(rect.size.height)
}

fn mid_x(rect: Bounds<Pixels>) -> f32 {
    f32::from(rect.center().x)
}

fn mid_y(rect: Bounds<Pixels>) -> f32 {
    f32::from(rect.center().y)
}

#[test]
fn fill_rect_covers_the_clip_at_the_content_aspect() {
    let clip = rect(10.0, 200.0, 128.0, 150.0);
    for aspect in [0.5, 128.0 / 150.0, 1.0, 16.0 / 9.0] {
        let fill = SharedElementGeometry::fill_rect(aspect, clip);
        assert!(is_close(width(fill) / height(fill), aspect, UNIT_TOLERANCE));
        assert!(min_x(fill) <= min_x(clip) + PIXEL_TOLERANCE && max_x(fill) >= max_x(clip) - PIXEL_TOLERANCE);
        assert!(min_y(fill) <= min_y(clip) + PIXEL_TOLERANCE && max_y(fill) >= max_y(clip) - PIXEL_TOLERANCE);
        assert!(is_close(mid_x(fill), mid_x(clip), PIXEL_TOLERANCE) && is_close(mid_y(fill), mid_y(clip), PIXEL_TOLERANCE));
    }
}

#[test]
fn fit_rect_stays_inside_the_clip_at_the_content_aspect() {
    let clip = rect(0.0, 120.0, 390.0, 520.0);
    for aspect in [0.5, 0.75, 1.0, 16.0 / 9.0] {
        let fit = SharedElementGeometry::fit_rect(aspect, clip);
        assert!(is_close(width(fit) / height(fit), aspect, UNIT_TOLERANCE));
        assert!(min_x(fit) >= min_x(clip) - PIXEL_TOLERANCE && max_x(fit) <= max_x(clip) + PIXEL_TOLERANCE);
        assert!(min_y(fit) >= min_y(clip) - PIXEL_TOLERANCE && max_y(fit) <= max_y(clip) + PIXEL_TOLERANCE);
        assert!(
            is_close(width(fit), width(clip), PIXEL_TOLERANCE) || is_close(height(fit), height(clip), PIXEL_TOLERANCE)
        );
    }
}

#[test]
fn unusable_aspect_returns_the_clip() {
    let clip = rect(4.0, 8.0, 100.0, 50.0);
    let zero = rect(0.0, 0.0, 0.0, 0.0);
    assert_eq!(SharedElementGeometry::fill_rect(0.0, clip), clip);
    assert_eq!(SharedElementGeometry::fit_rect(f32::NAN, clip), clip);
    assert_eq!(SharedElementGeometry::fill_rect(1.0, zero), zero);
}

#[test]
fn lerped_fill_rects_keep_the_aspect() {
    let aspect: f32 = 4.0 / 3.0;
    let source = SharedElementGeometry::fill_rect(aspect, rect(131.0, 480.0, 128.67, 150.0));
    let target = SharedElementGeometry::fill_rect(aspect, rect(0.0, 118.0, 390.0, 468.0));
    for u in [0.0, 0.25, 0.5, 1.0] {
        let lerped = SharedElementGeometry::lerp_rect(source, target, u);
        assert!(is_close(width(lerped) / height(lerped), aspect, UNIT_TOLERANCE));
    }
}

#[test]
fn lerped_fill_contains_the_lerped_clip_at_every_sample() {
    let aspect: f32 = 0.8;
    let source_clip = rect(262.0, 632.0, 128.67, 150.0);
    let target_clip = rect(0.0, 110.0, 390.0, 488.0);
    let source_fill = SharedElementGeometry::fill_rect(aspect, source_clip);
    let target_fill = SharedElementGeometry::fill_rect(aspect, target_clip);

    for u in samples() {
        let clip = SharedElementGeometry::lerp_rect(source_clip, target_clip, u);
        let content = SharedElementGeometry::lerp_rect(source_fill, target_fill, u);
        assert!(min_x(content) <= min_x(clip) + PIXEL_TOLERANCE);
        assert!(min_y(content) <= min_y(clip) + PIXEL_TOLERANCE);
        assert!(max_x(content) >= max_x(clip) - PIXEL_TOLERANCE);
        assert!(max_y(content) >= max_y(clip) - PIXEL_TOLERANCE);
    }
}

#[test]
fn anchored_width_fit_pins_top_leading_at_the_clip_width() {
    let clip = rect(16.0, 300.0, 358.0, 90.0);
    let fitted = SharedElementGeometry::anchored_width_fit_rect(size(px(128.0), px(150.0)), clip);
    assert_eq!(min_x(fitted), min_x(clip));
    assert_eq!(min_y(fitted), min_y(clip));
    assert_eq!(width(fitted), width(clip));
    assert!(is_close(height(fitted), 358.0 * 150.0 / 128.0, PIXEL_TOLERANCE));
}

#[test]
fn smoothstep_clamps_and_hits_the_ends() {
    assert_eq!(SharedElementGeometry::smoothstep(-1.0), 0.0);
    assert_eq!(SharedElementGeometry::smoothstep(0.0), 0.0);
    assert_eq!(SharedElementGeometry::smoothstep(0.5), 0.5);
    assert_eq!(SharedElementGeometry::smoothstep(1.0), 1.0);
    assert_eq!(SharedElementGeometry::smoothstep(2.0), 1.0);
}

#[test]
fn rect_center_and_edge_delta() {
    let centred = SharedElementGeometry::rect(point(px(100.0), px(200.0)), size(px(40.0), px(20.0)));
    assert_eq!(centred, rect(80.0, 190.0, 40.0, 20.0));

    let moved = Bounds::new(point(centred.origin.x + px(0.5), centred.origin.y - px(3.0)), centred.size);
    assert!(is_close(SharedElementGeometry::max_edge_delta(centred, moved), 3.0, PIXEL_TOLERANCE));
    assert_eq!(SharedElementGeometry::max_edge_delta(centred, centred), 0.0);
    let null = rect(f32::INFINITY, f32::INFINITY, 0.0, 0.0);
    assert_eq!(SharedElementGeometry::max_edge_delta(centred, null), f32::INFINITY);
}

#[test]
fn curve_time_inverts_progress() {
    let curve = SharedElementMotion::CURVE;
    for t in samples() {
        let u = curve.progress(t);
        assert!(is_close(curve.time(u), t, 1e-4));
    }
}

#[test]
fn curve_progress_inverts_time() {
    let curve = SharedElementMotion::CURVE;
    for u in samples() {
        let t = curve.time(u);
        assert!(is_close(curve.progress(t), u, UNIT_TOLERANCE));
    }
}

#[test]
fn curve_progress_is_monotone_and_pinned_at_the_ends() {
    let curve = SharedElementMotion::CURVE;
    assert_eq!(curve.progress(0.0), 0.0);
    assert_eq!(curve.progress(1.0), 1.0);
    assert_eq!(curve.time(0.0), 0.0);
    assert_eq!(curve.time(1.0), 1.0);

    let mut previous: f32 = 0.0;
    for t in samples() {
        let u = curve.progress(t);
        assert!(u >= previous - UNIT_TOLERANCE);
        previous = u;
    }
}

#[test]
fn linear_curve_is_the_identity() {
    let linear = SharedElementCubicTiming {
        c1: point(1.0 / 3.0, 1.0 / 3.0),
        c2: point(2.0 / 3.0, 2.0 / 3.0),
    };
    for t in samples() {
        assert!(is_close(linear.progress(t), t, UNIT_TOLERANCE));
    }
}

#[test]
fn cover_keyframes_fade_in_after_the_start_progress() {
    let curve = SharedElementMotion::CURVE;
    let start = SharedElementMotion::COVER_FADE_START_PROGRESS;
    let sample_count = SharedElementMotion::COVER_KEYFRAME_SAMPLES;
    let (values, key_times) = SharedElementCoverSchedule::opacity_keyframes(curve, start, sample_count);

    assert_eq!(values.len(), sample_count + 1);
    assert_eq!(key_times.len(), values.len());
    assert_eq!(values.first(), Some(&0.0));
    assert_eq!(values.last(), Some(&1.0));
    assert_eq!(key_times.first(), Some(&0.0));
    assert_eq!(key_times.last(), Some(&1.0));
    assert_eq!(values[1], 0.0);
    assert!(is_close(key_times[1], curve.time(start), UNIT_TOLERANCE));

    for pair in values.windows(2) {
        assert!(pair[1] >= pair[0]);
    }
    for pair in key_times.windows(2) {
        assert!(pair[1] >= pair[0]);
    }
}

#[test]
fn cover_keyframes_without_delay_start_at_zero() {
    let (values, key_times) = SharedElementCoverSchedule::opacity_keyframes(SharedElementMotion::CURVE, 0.0, 1);
    assert_eq!(values, vec![0.0, 1.0]);
    assert_eq!(key_times, vec![0.0, 1.0]);
}
