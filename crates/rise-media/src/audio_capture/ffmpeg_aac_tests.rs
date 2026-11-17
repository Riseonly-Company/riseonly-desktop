use super::audio_capture::{AacQuality, AacSettings, AudioCaptureError};
use super::ffmpeg_aac::*;

#[test]
fn a_voice_message_keeps_its_32_kbit_rate() {
    let plan = FfmpegAacPlan::for_settings(&AacSettings::voice_message()).expect("voice message plan");
    assert_eq!(
        plan,
        FfmpegAacPlan {
            sample_rate: 44_100,
            channels: 1,
            bit_rate: 32_000,
        }
    );
}

#[test]
fn a_video_note_without_a_rate_takes_the_high_quality_rate() {
    let plan = FfmpegAacPlan::for_settings(&AacSettings::video_note()).expect("video note plan");
    assert_eq!(plan.bit_rate, HIGH_BIT_RATE_PER_CHANNEL);
}

#[test]
fn a_rate_left_to_the_quality_scales_with_the_channels() {
    let stereo_medium = AacSettings {
        sample_rate: 48_000,
        channels: 2,
        bit_rate: None,
        quality: AacQuality::Medium,
    };
    assert_eq!(ffmpeg_bit_rate(&stereo_medium), 2 * MEDIUM_BIT_RATE_PER_CHANNEL);
}

#[test]
fn invalid_settings_are_refused_before_ffmpeg_sees_them() {
    let odd_rate = AacSettings {
        sample_rate: 44_000,
        ..AacSettings::voice_message()
    };
    assert!(matches!(
        FfmpegAacPlan::for_settings(&odd_rate),
        Err(AudioCaptureError::InvalidSettings(_))
    ));
}

#[test]
fn the_reason_names_what_the_build_lacks() {
    assert_eq!(missing_component_reason(true, true), None);
    let no_encoder = missing_component_reason(false, true).expect("reason");
    assert!(no_encoder.contains("\"aac\" encoder"));
    assert!(!no_encoder.contains("muxer\""));
    assert!(no_encoder.contains(FFMPEG_CONFIGURE_HINT));
    let no_muxer = missing_component_reason(true, false).expect("reason");
    assert!(no_muxer.contains("\"ipod\" muxer"));
    let neither = missing_component_reason(false, false).expect("reason");
    assert!(neither.contains("encoder and the"));
}

#[test]
fn stereo_samples_split_into_planes_of_one_aac_frame() {
    let mut frames = PlanarFrames::new(2, 4);
    frames.push(&[0.1, -0.1, 0.2, -0.2, 0.3, -0.3]);
    assert_eq!(frames.pending_frames(), 3);
    assert!(frames.pop_full().is_none());
    frames.push(&[0.4, -0.4, 0.5, -0.5]);
    let first = frames.pop_full().expect("full frame");
    assert_eq!(first.frames, 4);
    assert_eq!(first.pts, 0);
    assert_eq!(first.planes, vec![vec![0.1, 0.2, 0.3, 0.4], vec![-0.1, -0.2, -0.3, -0.4]]);
    assert!(frames.pop_full().is_none());
    let last = frames.pop_last().expect("short last frame");
    assert_eq!(last.frames, 1);
    assert_eq!(last.pts, 4);
    assert_eq!(last.planes, vec![vec![0.5], vec![-0.5]]);
    assert!(frames.pop_last().is_none());
    assert_eq!(frames.emitted_frames(), 5);
}

#[test]
fn samples_out_of_range_clip_and_nan_is_silence() {
    let mut frames = PlanarFrames::new(1, 3);
    frames.push(&[1.5, -2.0, f32::NAN]);
    let frame = frames.pop_full().expect("frame");
    assert_eq!(frame.planes, vec![vec![1.0, -1.0, 0.0]]);
}

#[test]
fn an_empty_take_still_gets_one_frame_of_silence() {
    let mut frames = PlanarFrames::new(1, AAC_FRAME_LENGTH);
    let silence = frames.pop_last().expect("silent frame");
    assert_eq!(silence.frames, AAC_FRAME_LENGTH);
    assert!(silence.planes[0].iter().all(|sample| *sample == 0.0));
    assert!(frames.pop_last().is_none());
}

#[test]
fn an_encoder_without_a_fixed_frame_size_gets_one_aac_frame() {
    assert_eq!(PlanarFrames::new(1, 0).frame_length(), AAC_FRAME_LENGTH);
}

#[test]
fn a_trailing_partial_interleaved_frame_is_dropped() {
    let mut frames = PlanarFrames::new(2, 8);
    frames.push(&[0.1, 0.2, 0.3]);
    assert_eq!(frames.pending_frames(), 1);
}

#[test]
fn the_feature_off_reason_names_the_feature() {
    assert!(feature_off_reason().contains("ffmpeg feature"));
}
