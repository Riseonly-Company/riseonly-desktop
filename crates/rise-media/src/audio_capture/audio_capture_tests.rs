use std::time::Duration;

use crate::audio::format::StreamFormat;
use crate::audio_capture::*;
use rise_platform::HostOs;

#[derive(Default)]
struct RecordingWriter {
    samples: Vec<f32>,
    fail_after: Option<usize>,
}

impl AacFileWriter for RecordingWriter {
    fn write(&mut self, interleaved: &[f32]) -> Result<(), AudioCaptureError> {
        if let Some(limit) = self.fail_after
            && self.samples.len() + interleaved.len() > limit
        {
            return Err(AudioCaptureError::EncoderFailed("full".to_owned()));
        }
        self.samples.extend_from_slice(interleaved);
        Ok(())
    }

    fn finish(self: Box<Self>) -> Result<(), AudioCaptureError> {
        Ok(())
    }
}

#[test]
fn voice_message_settings_match_the_ios_recorder() {
    let voice = AacSettings::voice_message();
    assert_eq!(voice.sample_rate, 44_100);
    assert_eq!(voice.channels, 1);
    assert_eq!(voice.bit_rate, Some(32_000));
    assert_eq!(voice.quality, AacQuality::Medium);
    assert_eq!(voice.validate(), Ok(()));

    let note = AacSettings::video_note();
    assert_eq!(note.bit_rate, None);
    assert_eq!(note.quality, AacQuality::High);
    assert_eq!(note.validate(), Ok(()));
}

#[test]
fn quality_raw_values_are_av_audio_quality() {
    assert_eq!(AacQuality::Medium.raw(), 0x40);
    assert_eq!(AacQuality::High.raw(), 0x60);
}

#[test]
fn settings_outside_aac_lc_are_rejected() {
    let mut settings = AacSettings::voice_message();
    settings.sample_rate = 96_000;
    assert!(matches!(settings.validate(), Err(AudioCaptureError::InvalidSettings(_))));

    let mut settings = AacSettings::voice_message();
    settings.channels = 3;
    assert!(matches!(settings.validate(), Err(AudioCaptureError::InvalidSettings(_))));

    let mut settings = AacSettings::voice_message();
    settings.bit_rate = Some(4_000);
    assert!(matches!(settings.validate(), Err(AudioCaptureError::InvalidSettings(_))));
}

#[test]
fn file_writer_rejects_settings_before_the_encoder() {
    let mut settings = AacSettings::voice_message();
    settings.channels = 3;
    let path = std::env::temp_dir().join("rise-media-never-written.m4a");
    assert!(matches!(aac_file_writer(&path, &settings), Err(AudioCaptureError::InvalidSettings(_))));
    assert!(!path.exists());
}

#[test]
fn duration_counts_frames_at_the_take_rate() {
    let settings = AacSettings::voice_message();
    assert_eq!(settings.duration_of(44_100), Duration::from_secs(1));
    assert_eq!(settings.duration_of(0), Duration::ZERO);
}

#[test]
fn normalized_power_maps_minus_55_db_to_an_empty_bar() {
    assert_eq!(normalized_power(-160.0), 0.0);
    assert_eq!(normalized_power(METER_FLOOR_DB), 0.0);
    assert_eq!(normalized_power(0.0), 1.0);
    assert_eq!(normalized_power(6.0), 1.0);
    assert!((normalized_power(-27.5) - 0.5).abs() < 1e-6);
}

#[test]
fn meter_answers_silence_without_samples_and_resets_on_read() {
    let mut meter = PowerMeter::default();
    assert_eq!(meter.take_average_db(), SILENCE_DB);

    meter.add(&[1.0, -1.0, 1.0, -1.0]);
    assert!(meter.take_average_db().abs() < 1e-4);
    assert_eq!(meter.take_average_db(), SILENCE_DB);

    meter.add(&[0.0; 64]);
    assert_eq!(meter.take_average_db(), SILENCE_DB);

    meter.add(&[0.5; 64]);
    let half = meter.take_average_db();
    assert!((half - (-6.0206)).abs() < 1e-3, "{half}");
}

#[test]
fn remix_averages_to_mono_and_copies_mono_to_stereo() {
    let mut out = Vec::new();
    remix(&[1.0, 0.0, 0.5, 0.5], 2, 1, &mut out);
    assert_eq!(out, vec![0.5, 0.5]);

    out.clear();
    remix(&[0.25, 0.75], 1, 2, &mut out);
    assert_eq!(out, vec![0.25, 0.25, 0.75, 0.75]);

    out.clear();
    remix(&[0.1, 0.2, 0.3, 0.4, 0.5, 0.6], 3, 2, &mut out);
    assert_eq!(out, vec![0.1, 0.2, 0.4, 0.5]);

    out.clear();
    remix(&[0.1, 0.2, 0.3], 2, 1, &mut out);
    assert_eq!(out.len(), 1, "a trailing partial frame is dropped");
}

#[test]
fn pipeline_at_the_take_rate_writes_mixed_frames_and_meters_them() {
    let settings = AacSettings::voice_message();
    let input = StreamFormat {
        sample_rate: 44_100,
        channels: 2,
    };
    let mut pipeline = CapturePipeline::new(input, &settings);
    let mut meter = PowerMeter::default();
    let mut writer = RecordingWriter::default();

    let frames = pipeline
        .feed(&[0.5, 0.5, 0.5, 0.5, 0.5, 0.5], &mut meter, &mut writer)
        .expect("written");

    assert_eq!(frames, 3);
    assert_eq!(writer.samples, vec![0.5, 0.5, 0.5]);
    assert!((meter.take_average_db() - (-6.0206)).abs() < 1e-3);
}

#[test]
fn pipeline_resamples_a_48k_microphone_to_44_1k() {
    let settings = AacSettings::voice_message();
    let input = StreamFormat {
        sample_rate: 48_000,
        channels: 1,
    };
    let mut pipeline = CapturePipeline::new(input, &settings);
    let mut meter = PowerMeter::default();
    let mut writer = RecordingWriter::default();

    let mut written = 0;
    for _ in 0..10 {
        written += pipeline
            .feed(&[0.25; 4_800], &mut meter, &mut writer)
            .expect("written");
    }

    assert!((44_000..=44_100).contains(&written), "{written}");
    assert_eq!(writer.samples.len() as u64, written);
}

#[test]
fn pipeline_reports_an_encoder_refusal() {
    let settings = AacSettings::voice_message();
    let input = StreamFormat {
        sample_rate: 44_100,
        channels: 1,
    };
    let mut pipeline = CapturePipeline::new(input, &settings);
    let mut meter = PowerMeter::default();
    let mut writer = RecordingWriter {
        samples: Vec::new(),
        fail_after: Some(2),
    };

    let refused = pipeline.feed(&[0.1, 0.2, 0.3], &mut meter, &mut writer);
    assert!(matches!(refused, Err(AudioCaptureError::EncoderFailed(_))));
}

#[test]
fn unsupported_errors_are_the_ones_that_hide_the_button() {
    for host in [HostOs::MacOs, HostOs::Linux, HostOs::Windows] {
        assert!(AudioCaptureError::BranchNotWritten(host).is_unsupported());
    }
    assert!(AudioCaptureError::EncoderUnavailable("no aac".to_owned()).is_unsupported());
    assert!(!AudioCaptureError::NoInputDevice.is_unsupported());
}

#[test]
fn support_and_reason_agree() {
    let reason = recording_unavailable_reason();
    match recording_support() {
        rise_platform::PlatformSupport::Performed => assert_eq!(reason, None),
        rise_platform::PlatformSupport::Unsupported => {
            assert!(reason.is_some_and(|reason| reason.is_unsupported()));
        }
    }
}
