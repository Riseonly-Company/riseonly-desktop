use std::path::PathBuf;

use gpui::{px, size};

use super::*;

#[test]
fn frame_durations_fall_back_and_clamp() {
    assert_eq!(
        AnimatedImagePlaybackPolicy::normalized_frame_duration(None),
        AnimatedImagePlaybackPolicy::DEFAULT_FRAME_DURATION
    );
    assert_eq!(
        AnimatedImagePlaybackPolicy::normalized_frame_duration(Some(0.0)),
        AnimatedImagePlaybackPolicy::DEFAULT_FRAME_DURATION
    );
    assert_eq!(
        AnimatedImagePlaybackPolicy::normalized_frame_duration(Some(f64::NAN)),
        AnimatedImagePlaybackPolicy::DEFAULT_FRAME_DURATION
    );
    assert_eq!(
        AnimatedImagePlaybackPolicy::normalized_frame_duration(Some(0.001)),
        AnimatedImagePlaybackPolicy::MINIMUM_FRAME_DURATION
    );
    assert_eq!(AnimatedImagePlaybackPolicy::normalized_frame_duration(Some(5.0)), 1.0);
    assert_eq!(AnimatedImagePlaybackPolicy::normalized_frame_duration(Some(0.25)), 0.25);
}

#[test]
fn sources_resolve_to_local_files_or_the_network() {
    assert_eq!(AnimatedImageSourcePolicy::resolve("   "), None);
    assert_eq!(
        AnimatedImageSourcePolicy::resolve(" /tmp/a/../b.gif "),
        Some(AnimatedImageSourcePolicySource::LocalFile(PathBuf::from("/tmp/b.gif")))
    );
    assert_eq!(
        AnimatedImageSourcePolicy::resolve("https://cdn.example.com/a.gif"),
        Some(AnimatedImageSourcePolicySource::Remote)
    );
    assert_eq!(
        AnimatedImageSourcePolicy::resolve("HTTP://cdn.example.com/a.gif"),
        Some(AnimatedImageSourcePolicySource::Remote)
    );
    assert_eq!(AnimatedImageSourcePolicy::resolve("ftp://cdn.example.com/a.gif"), None);
}

#[test]
fn a_local_file_over_the_payload_limit_is_refused() {
    let missing = PathBuf::from("/nonexistent/animated-gif-view-test.gif");
    assert_eq!(AnimatedImageSourcePolicy::mapped_local_data(&missing, 1_024), None);
    assert_eq!(AnimatedImageSourcePolicy::mapped_local_data(&missing, 0), None);
}

#[test]
fn a_still_or_broken_payload_is_not_a_complete_animation() {
    assert!(!AnimatedImageSourcePolicy::is_complete_animation(&[]));
    assert!(!AnimatedImageSourcePolicy::is_complete_animation(b"not an image"));
}

#[test]
fn decode_budget_follows_the_box_edge_that_content_mode_scales_by() {
    let fill = DecodeBudget {
        box_point_size: size(px(300.), px(100.)),
        display_scale: 2.0,
        fills_box: true,
        produces_backdrop: false,
    };
    assert_eq!(fill.maximum_pixel_dimension((200, 200)), 600);

    let fit = DecodeBudget { fills_box: false, ..fill };
    assert_eq!(fit.maximum_pixel_dimension((200, 200)), 200);
}

#[test]
fn decode_budget_stays_inside_its_floor_and_ceiling() {
    let budget = DecodeBudget {
        box_point_size: size(px(4_000.), px(4_000.)),
        display_scale: 3.0,
        fills_box: true,
        produces_backdrop: false,
    };
    assert_eq!(budget.maximum_pixel_dimension((100, 100)), DecodeBudget::CEILING);

    let tiny = DecodeBudget { box_point_size: size(px(4.), px(4.)), ..budget };
    assert_eq!(tiny.maximum_pixel_dimension((100, 100)), DecodeBudget::MINIMUM);

    let unknown = DecodeBudget { box_point_size: size(px(0.), px(0.)), ..budget };
    assert_eq!(unknown.maximum_pixel_dimension((100, 100)), DecodeBudget::CEILING);
}

#[test]
fn payload_keys_round_trip_to_their_source() {
    let key = AnimatedImageSourcePolicy::payload_cache_key("https://cdn.example.com/a.gif");
    assert_eq!(
        AnimatedImageSourcePolicy::source_key(&key).as_deref(),
        Some("https://cdn.example.com/a.gif")
    );
    assert_eq!(AnimatedImageSourcePolicy::source_key("https://cdn.example.com/a.gif"), None);
}

#[test]
fn rendered_frames_are_bounded_and_keep_their_aspect() {
    let buffer = RgbaImage::from_pixel(400, 200, image::Rgba([10, 20, 30, 255]));
    let image = render_image(buffer, 100);
    let pixels = image.size(0);
    assert_eq!((pixels.width.0, pixels.height.0), (100, 50));
}
