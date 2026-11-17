use super::rise_lottie_frame_cache::RiseLottieRasterPolicy;

#[test]
fn stride_keeps_authored_duration_for_fast_sources() {
    let full_size = RiseLottieRasterPolicy::MAXIMUM_DIMENSION * RiseLottieRasterPolicy::MAXIMUM_DIMENSION;
    assert_eq!(RiseLottieRasterPolicy::presentation_stride(60.0, full_size), 1);
    assert_eq!(RiseLottieRasterPolicy::presentation_stride(30.0, full_size), 1);
    assert_eq!(RiseLottieRasterPolicy::presentation_stride(120.0, full_size), 2);
}

#[test]
fn small_art_spends_pixel_budget_on_authored_rate() {
    assert_eq!(RiseLottieRasterPolicy::presentation_stride(60.0, 128 * 128), 1);
    assert_eq!(RiseLottieRasterPolicy::presentation_stride(120.0, 128 * 128), 1);
}

#[test]
fn stride_never_stalls_slow_or_degenerate_sources() {
    assert_eq!(RiseLottieRasterPolicy::presentation_stride(24.0, 128 * 128), 1);
    assert_eq!(RiseLottieRasterPolicy::presentation_stride(0.0, 128 * 128), 1);
    assert_eq!(RiseLottieRasterPolicy::presentation_stride(-5.0, 128 * 128), 1);
}

#[test]
fn dimension_is_bounded_and_proportional() {
    assert_eq!(RiseLottieRasterPolicy::dimension(20.0), 40);
    assert_eq!(RiseLottieRasterPolicy::dimension(74.0), 148);
    assert_eq!(RiseLottieRasterPolicy::dimension(180.0), 256);
    assert_eq!(RiseLottieRasterPolicy::dimension(4.0), 32);
    assert_eq!(RiseLottieRasterPolicy::dimension(0.0), 32);
}

#[test]
fn per_sequence_retention_is_bounded() {
    assert!(RiseLottieRasterPolicy::maximum_bytes_per_sequence() < RiseLottieRasterPolicy::maximum_cache_bytes());
    let largest_frame_bytes =
        RiseLottieRasterPolicy::dimension(180.0) * RiseLottieRasterPolicy::dimension(180.0) * 4;
    assert!(RiseLottieRasterPolicy::maximum_bytes_per_sequence() >= largest_frame_bytes);
}

#[test]
fn render_concurrency_leaves_headroom() {
    let concurrency = RiseLottieRasterPolicy::render_concurrency();
    assert!(concurrency >= 1);
    assert!(concurrency <= 3);
}
