use gpui::hsla;

use crate::scroll_chrome_shadow_ui::ScrollChromeShadowStyle;
use crate::scroll_edge_blur_ui::ScrollEdgeBlurFalloff;

#[test]
fn tint_never_reaches_an_opaque_band() {
    let page = hsla(0.0, 0.0, 0.05, 1.0);
    for solid_end in [0.0, ScrollEdgeBlurFalloff::DEFAULT_SOLID_END, 0.9] {
        let stops = ScrollEdgeBlurFalloff::tint_stops(page, solid_end, false);
        assert!(stops.iter().all(|stop| stop.color.a <= ScrollEdgeBlurFalloff::TINT_PEAK + f32::EPSILON));
        assert!(stops.last().is_some_and(|stop| stop.color.a < 0.01));
    }
}

#[test]
fn tint_without_plateau_thins_from_the_top_edge() {
    let page = hsla(0.0, 0.0, 0.05, 1.0);
    let stops = ScrollEdgeBlurFalloff::tint_stops(page, 0.0, false);
    assert!(stops.windows(2).all(|pair| pair[1].color.a < pair[0].color.a));
}

#[test]
fn reduced_transparency_keeps_the_ios_solid_branch() {
    let page = hsla(0.0, 0.0, 0.05, 1.0);
    let stops = ScrollEdgeBlurFalloff::tint_stops(page, 0.3, true);
    assert_eq!(stops.len(), 3);
    assert_eq!(stops[0].color.a, 1.0);
}

#[test]
fn list_scrims_leave_the_rows_readable() {
    for style in [
        ScrollChromeShadowStyle::list_shell_header(),
        ScrollChromeShadowStyle::list_shell_blurred_header(),
        ScrollChromeShadowStyle::floating_bottom_action(),
    ] {
        assert!(style.peak_opacity <= 0.7, "peak {}", style.peak_opacity);
        assert!(!style.opaque_through_header);
    }
    let blurred = ScrollChromeShadowStyle::list_shell_blurred_header();
    let combined = 1.0 - (1.0 - ScrollEdgeBlurFalloff::HEADER_TINT_PEAK as f64) * (1.0 - blurred.peak_opacity);
    assert!(combined < 0.95, "combined {combined}");
    assert!(ScrollEdgeBlurFalloff::HEADER_TINT_PEAK >= 0.85, "row text must not read under the title");
}

#[test]
fn chat_list_header_tint_holds_through_the_header_and_stays_translucent() {
    let page = hsla(0.0, 0.0, 0.05, 1.0);
    let stops = ScrollEdgeBlurFalloff::tint_stops_with_peak(page, 0.8, ScrollEdgeBlurFalloff::HEADER_TINT_PEAK, false);
    let held = stops.iter().filter(|stop| stop.location < 0.8);
    assert!(held.into_iter().all(|stop| (stop.color.a - ScrollEdgeBlurFalloff::HEADER_TINT_PEAK).abs() < 1e-4));
    assert!(stops.iter().all(|stop| stop.color.a < 1.0));
    assert!(stops.last().is_some_and(|stop| stop.color.a < 0.01));
}

#[test]
fn tall_header_stays_solid_behind_the_title() {
    let style = ScrollChromeShadowStyle::list_shell_tall_header();
    assert!(style.opaque_through_header);
    assert_eq!(style.peak_opacity, 1.0);
    assert_eq!(style.overflow_below_header, 14.0);
}

#[test]
fn stretched_band_runs_twice_as_long_and_keeps_the_plateau() {
    let (scale, solid_end) = ScrollEdgeBlurFalloff::stretched(0.6);
    assert!((scale - 1.4).abs() < 1e-5);
    assert!((solid_end * scale - 0.6).abs() < 1e-5);
    assert!((scale - solid_end * scale - ScrollEdgeBlurFalloff::BAND_STRETCH * 0.4).abs() < 1e-5);
}

#[test]
fn header_band_ends_where_the_header_and_its_fade_end() {
    let (height, solid_end) = ScrollEdgeBlurFalloff::header_band(56.0, 12.0);
    assert!((height - 68.0).abs() < 1e-5);
    assert!((solid_end * height - 56.0).abs() < 1e-4);
    assert_eq!(ScrollEdgeBlurFalloff::header_band(44.0, 0.0), (44.0, 1.0));
    assert_eq!(ScrollEdgeBlurFalloff::header_band(0.0, 0.0), (0.0, 0.0));
    assert_eq!(ScrollEdgeBlurFalloff::header_band(-3.0, 8.0), (8.0, 0.0));
}

#[test]
fn falloff_eases_in_from_the_content() {
    assert!(ScrollEdgeBlurFalloff::strength(0.9, 0.0) < 0.02);
    assert!(ScrollEdgeBlurFalloff::strength(0.5, 0.0) < 0.5);
    assert!((ScrollEdgeBlurFalloff::strength(0.3, 0.3) - 1.0).abs() < 1e-5);
}

#[test]
fn without_a_renderer_blur_the_edge_is_opaque_and_fades_to_nothing() {
    let color = hsla(0.0, 0.0, 0.1, 1.0);
    let stops = ScrollEdgeBlurFalloff::paint_stops(color, 0.0, 0.3, false, false);
    let first = stops.first().expect("stops");
    let last = stops.last().expect("stops");
    assert!(first.color.a > 0.98, "edge alpha {}", first.color.a);
    assert!(last.color.a < 0.01, "far end alpha {}", last.color.a);
    assert!(stops.windows(2).all(|pair| pair[1].color.a <= pair[0].color.a + f32::EPSILON));
}

#[test]
fn without_a_renderer_blur_the_header_plateau_stays_opaque() {
    let color = hsla(0.0, 0.0, 0.1, 1.0);
    let solid_end = 0.6;
    let stops = ScrollEdgeBlurFalloff::paint_stops(color, solid_end, 0.5, false, false);
    assert!(stops.iter().filter(|stop| stop.location <= solid_end).all(|stop| stop.color.a > 0.99));
}

#[test]
fn a_renderer_blur_keeps_the_translucent_tint() {
    let color = hsla(0.0, 0.0, 0.1, 1.0);
    let blurred = ScrollEdgeBlurFalloff::paint_stops(color, 0.0, 0.5, false, true);
    assert_eq!(blurred, ScrollEdgeBlurFalloff::tint_stops_with_peak(color, 0.0, 0.5, false));
}
