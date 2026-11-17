use std::time::Duration;

use crate::video::av_player::media_clock::*;

fn ms(value: u64) -> Duration {
    Duration::from_millis(value)
}

#[test]
fn a_rate_that_stops_the_player_has_no_tempo_and_others_are_clamped_to_atempo() {
    assert_eq!(tempo_of_rate(0.0), None);
    assert_eq!(tempo_of_rate(-1.0), None);
    assert_eq!(tempo_of_rate(f32::NAN), None);
    assert_eq!(tempo_of_rate(1.5), Some(1.5));
    assert_eq!(tempo_of_rate(2.0), Some(2.0));
    assert_eq!(tempo_of_rate(0.25), Some(MIN_TEMPO));
    assert_eq!(tempo_of_rate(500.0), Some(MAX_TEMPO));
}

#[test]
fn the_source_is_declared_faster_only_without_atempo() {
    assert_eq!(declared_source_rate(48_000, 1.5, true), 48_000);
    assert_eq!(declared_source_rate(48_000, 1.0, false), 48_000);
    assert_eq!(declared_source_rate(48_000, 1.5, false), 72_000);
    assert_eq!(declared_source_rate(44_100, 2.0, false), 88_200);
}

#[test]
fn the_filter_spec_names_atempo_only_for_another_tempo() {
    assert_eq!(
        audio_filter_spec(1.0, true, 48_000, 2),
        "aresample=48000,aformat=sample_fmts=flt:sample_rates=48000:channel_layouts=2c"
    );
    assert_eq!(
        audio_filter_spec(1.5, true, 44_100, 1),
        "atempo=1.5,aresample=44100,aformat=sample_fmts=flt:sample_rates=44100:channel_layouts=1c"
    );
    assert_eq!(
        audio_filter_spec(2.0, false, 48_000, 2),
        "aresample=48000,aformat=sample_fmts=flt:sample_rates=48000:channel_layouts=2c"
    );
}

#[test]
fn the_timeline_maps_played_frames_to_media_time_per_segment() {
    let mut timeline = AudioTimeline::new(48_000);
    assert_eq!(timeline.media_us_at(0), None);
    timeline.begin_segment(1_000_000, 1.0);
    timeline.pushed(48_000);
    assert_eq!(timeline.media_us_at(24_000), Some(1_500_000));
    timeline.begin_segment(2_000_000, 2.0);
    timeline.pushed(48_000);
    assert_eq!(timeline.media_us_at(48_000), Some(2_000_000));
    assert_eq!(timeline.media_us_at(72_000), Some(3_000_000));
    assert_eq!(timeline.end_us(), Some(4_000_000));
}

#[test]
fn a_segment_begun_twice_at_the_same_frame_keeps_the_newest() {
    let mut timeline = AudioTimeline::new(1_000);
    timeline.begin_segment(0, 1.0);
    timeline.begin_segment(500_000, 1.5);
    timeline.pushed(1_000);
    assert_eq!(timeline.media_us_at(1_000), Some(2_000_000));
}

#[test]
fn the_host_clock_stands_until_it_runs_and_keeps_its_time_across_tempos() {
    let mut clock = HostMediaClock::at(2_000_000, ms(100));
    assert_eq!(clock.media_us(ms(900)), 2_000_000);
    clock.run(1.0, ms(1_000));
    assert_eq!(clock.media_us(ms(1_500)), 2_500_000);
    clock.run(2.0, ms(1_500));
    assert_eq!(clock.media_us(ms(2_000)), 3_500_000);
    clock.stop(ms(2_000));
    assert!(!clock.is_running());
    assert_eq!(clock.media_us(ms(9_000)), 3_500_000);
}

#[test]
fn pictures_are_shown_dropped_or_waited_for_against_the_clock() {
    assert_eq!(media_picture_due(1_000_000, 1_000_000, 1.0, true), MediaPictureDue::Show);
    assert_eq!(media_picture_due(1_000_000, 1_050_000, 1.0, true), MediaPictureDue::Show);
    assert_eq!(media_picture_due(1_000_000, 1_200_000, 1.0, true), MediaPictureDue::Drop);
    assert_eq!(media_picture_due(1_040_000, 1_000_000, 2.0, false), MediaPictureDue::After(ms(20)));
    assert_eq!(media_picture_due(1_040_000, 1_000_000, 1.0, true), MediaPictureDue::After(AUDIO_CLOCK_POLL));
    assert_eq!(media_picture_due(9_000_000, 1_000_000, 1.0, false), MediaPictureDue::Show);
}

#[test]
fn item_times_never_leave_the_item() {
    assert_eq!(item_seconds(-5, Some(3_000_000)), 0.0);
    assert_eq!(item_seconds(4_000_000, Some(3_000_000)), 3.0);
    assert_eq!(item_seconds(1_500_000, None), 1.5);
    assert_eq!(duration_seconds(Some(2_500_000)), Some(2.5));
    assert_eq!(duration_seconds(Some(0)), None);
    assert_eq!(duration_seconds(None), None);
}
