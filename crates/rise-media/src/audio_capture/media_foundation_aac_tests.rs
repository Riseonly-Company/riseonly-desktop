use super::audio_capture::{AAC_SAMPLE_RATES, AacQuality, AacSettings, AudioCaptureError};
use super::media_foundation_aac::*;

#[test]
fn a_voice_message_is_mono_44_1_khz_at_the_lowest_windows_rate() {
    let format = MfAacFormat::for_settings(&AacSettings::voice_message()).expect("voice message format");
    assert_eq!(
        format,
        MfAacFormat {
            take_rate: 44_100,
            sample_rate: 44_100,
            channels: 1,
            aac_bytes_per_second: 12_000,
            pcm_block_alignment: 2,
            pcm_bytes_per_second: 88_200,
        }
    );
}

#[test]
fn a_video_note_without_a_rate_takes_the_high_quality_step() {
    let format = MfAacFormat::for_settings(&AacSettings::video_note()).expect("video note format");
    assert_eq!(format.aac_bytes_per_second, 16_000);
    assert_eq!(format.channels, 1);
}

#[test]
fn medium_quality_without_a_rate_is_the_encoder_default() {
    let settings = AacSettings {
        bit_rate: None,
        quality: AacQuality::Medium,
        ..AacSettings::voice_message()
    };
    assert_eq!(aac_bytes_per_second(&settings), 12_000);
}

#[test]
fn a_fixed_rate_rounds_up_to_the_next_rate_the_encoder_accepts() {
    let rate = |bits| {
        aac_bytes_per_second(&AacSettings {
            bit_rate: Some(bits),
            ..AacSettings::voice_message()
        })
    };
    assert_eq!(rate(8_000), 12_000);
    assert_eq!(rate(96_000), 12_000);
    assert_eq!(rate(96_001), 16_000);
    assert_eq!(rate(128_000), 16_000);
    assert_eq!(rate(160_000), 20_000);
    assert_eq!(rate(192_000), 24_000);
    assert_eq!(rate(320_000), 24_000);
}

#[test]
fn stereo_48_khz_doubles_the_pcm_frame() {
    let settings = AacSettings {
        sample_rate: 48_000,
        channels: 2,
        ..AacSettings::voice_message()
    };
    let format = MfAacFormat::for_settings(&settings).expect("stereo format");
    assert_eq!(format.pcm_block_alignment, 4);
    assert_eq!(format.pcm_bytes_per_second, 192_000);
}

#[test]
fn a_rate_below_44_1_khz_is_encoded_at_44_1_khz_and_resampled() {
    let settings = AacSettings {
        sample_rate: 22_050,
        channels: 2,
        ..AacSettings::voice_message()
    };
    let format = MfAacFormat::for_settings(&settings).expect("22.05 kHz is an AAC-LC rate");
    assert_eq!(format.take_rate, 22_050);
    assert_eq!(format.sample_rate, 44_100);
    assert!(format.resamples());
    assert_eq!(format.pcm_bytes_per_second, 44_100 * 4);
}

#[test]
fn every_aac_lc_rate_maps_to_the_lowest_encoder_rate_not_below_it() {
    for rate in AAC_SAMPLE_RATES {
        let encoded = media_foundation_rate(rate);
        assert!(MF_AAC_SAMPLE_RATES.contains(&encoded), "{rate} -> {encoded}");
        assert!(encoded >= rate, "{rate} -> {encoded}");
    }
    assert_eq!(media_foundation_rate(32_000), 44_100);
    assert_eq!(media_foundation_rate(44_100), 44_100);
    assert_eq!(media_foundation_rate(48_000), 48_000);
}

#[test]
fn an_encoder_rate_is_written_as_is() {
    let format = MfAacFormat::for_settings(&AacSettings::voice_message()).expect("voice format");
    assert_eq!(format.take_rate, format.sample_rate);
    assert!(!format.resamples());
}

#[test]
fn settings_the_seam_rejects_stay_rejected() {
    let settings = AacSettings {
        channels: 3,
        ..AacSettings::voice_message()
    };
    assert!(matches!(
        MfAacFormat::for_settings(&settings),
        Err(AudioCaptureError::InvalidSettings(_))
    ));
}

#[test]
fn float_samples_become_little_endian_16_bit_with_clipping() {
    let mut out = Vec::new();
    append_pcm16(&[0.0, 1.0, -1.0, 2.0, -2.0, 0.5, f32::NAN], &mut out);
    let samples: Vec<i16> = out
        .chunks_exact(2)
        .map(|pair| i16::from_le_bytes([pair[0], pair[1]]))
        .collect();
    assert_eq!(samples, vec![0, 32_767, -32_767, 32_767, -32_767, 16_384, 0]);
}

#[test]
fn appending_keeps_what_the_buffer_already_holds() {
    let mut out = vec![0xAB];
    append_pcm16(&[0.0], &mut out);
    assert_eq!(out, vec![0xAB, 0, 0]);
}

#[test]
fn the_clock_never_drifts_and_every_duration_is_nonzero() {
    let mut clock = SampleClock::new(44_100);
    let mut expected_start = 0;
    for _ in 0..1_000 {
        let (time, duration) = clock.advance(441);
        assert_eq!(time, expected_start);
        assert!(duration > 0);
        expected_start = time + duration;
    }
    assert_eq!(clock.frames(), 441_000);
    assert_eq!(expected_start, 10 * HNS_PER_SECOND as i64);
}

#[test]
fn an_uneven_block_rounds_inside_the_take_not_across_it() {
    let mut clock = SampleClock::new(48_000);
    let (first_time, first_duration) = clock.advance(1);
    let (second_time, second_duration) = clock.advance(1);
    let (third_time, third_duration) = clock.advance(1);
    assert_eq!((first_time, first_duration), (0, 208));
    assert_eq!((second_time, second_duration), (208, 208));
    assert_eq!((third_time, third_duration), (416, 209));
}

#[test]
fn the_padding_frame_is_one_aac_frame_of_silence_per_channel() {
    assert_eq!(silent_aac_frame(1).len(), AAC_FRAME_LENGTH);
    assert_eq!(silent_aac_frame(2).len(), 2 * AAC_FRAME_LENGTH);
    assert!(silent_aac_frame(2).iter().all(|sample| *sample == 0.0));
}
