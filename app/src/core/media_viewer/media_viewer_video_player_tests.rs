use rise_media::video::av_player::CMTimeRange;

use crate::core::media::video_warm_start_prefetch::VideoWarmStartPolicy;
use crate::core::media_viewer::media_viewer_video_player::MediaViewerVideoBufferMetrics;
use crate::core::ui::animated_media_playback_coordinator::AnimatedMediaThermalLevel;

#[test]
fn merges_only_ranges_contiguous_with_the_playhead() {
    let metrics = MediaViewerVideoBufferMetrics::resolve(
        &[CMTimeRange::new(0.0, 8.0), CMTimeRange::new(8.05, 4.0), CMTimeRange::new(15.0, 3.0)],
        5.0,
        20.0,
    );

    assert!((metrics.buffered_until - 12.05).abs() < 0.001);
    assert!((metrics.buffered_ahead - 7.05).abs() < 0.001);
    assert!((metrics.fraction.unwrap_or(0.0) - 0.6025).abs() < 0.0001);
}

#[test]
fn does_not_present_determinate_progress_without_known_duration() {
    let metrics = MediaViewerVideoBufferMetrics::resolve(&[CMTimeRange::new(0.0, 5.0)], 1.0, f64::NAN);

    assert_eq!(metrics.buffered_ahead, 4.0);
    assert_eq!(metrics.fraction, None);
}

#[test]
fn clamps_buffer_fraction_at_completion() {
    let metrics = MediaViewerVideoBufferMetrics::resolve(&[CMTimeRange::new(0.0, 12.0)], 2.0, 10.0);

    assert_eq!(metrics.fraction, Some(1.0));
}

#[test]
fn warm_start_only_runs_when_runtime_pressure_allows_it() {
    assert!(VideoWarmStartPolicy::allows_prefetch(true, false, AnimatedMediaThermalLevel::Nominal));
    assert!(VideoWarmStartPolicy::allows_prefetch(true, false, AnimatedMediaThermalLevel::Fair));
    assert!(!VideoWarmStartPolicy::allows_prefetch(true, true, AnimatedMediaThermalLevel::Nominal));
    assert!(!VideoWarmStartPolicy::allows_prefetch(true, false, AnimatedMediaThermalLevel::Serious));
    assert!(!VideoWarmStartPolicy::allows_prefetch(false, false, AnimatedMediaThermalLevel::Nominal));
    assert_eq!(VideoWarmStartPolicy::PREFERRED_BUFFER_DURATION.as_secs_f64(), 8.0);
}
