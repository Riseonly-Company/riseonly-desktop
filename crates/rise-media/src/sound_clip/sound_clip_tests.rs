use std::sync::Arc;

use super::sound_clip::*;
use crate::audio::format::SampleFormat;
use crate::audio::output::DeviceConfig;

/// builds the wav for the end tone, canonical 44 byte header and 16 bit pcm
fn wav(sample_rate: u32, channels: u16, samples: &[i16]) -> Vec<u8> {
    let mut pcm = Vec::with_capacity(samples.len() * 2);
    for sample in samples {
        pcm.extend_from_slice(&sample.to_le_bytes());
    }

    let mut out = Vec::new();
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + pcm.len() as u32).to_le_bytes());
    out.extend_from_slice(b"WAVE");
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&channels.to_le_bytes());
    out.extend_from_slice(&sample_rate.to_le_bytes());
    out.extend_from_slice(&(sample_rate * channels as u32 * 2).to_le_bytes());
    out.extend_from_slice(&(channels * 2).to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&(pcm.len() as u32).to_le_bytes());
    out.extend_from_slice(&pcm);
    out
}

fn device(sample_rate: u32, channels: u16) -> DeviceConfig {
    DeviceConfig {
        sample_rate,
        channels,
        sample_format: SampleFormat::F32,
        resampling: false,
    }
}

#[test]
fn the_end_tone_header_is_read_as_mono_48k() {
    let samples = vec![0i16; 27_840];
    let clip = SoundClip::from_wav(wav(48_000, 1, &samples).into()).unwrap();

    assert_eq!(clip.format().sample_rate, 48_000);
    assert_eq!(clip.format().channels, 1);
    assert_eq!(clip.frames(), 27_840);
    assert_eq!(clip.duration_millis(), 580);
}

#[test]
fn samples_are_scaled_from_i16_to_unit_floats() {
    let data = wav(44_100, 2, &[i16::MAX, -i16::MAX, 0, 16_384]);
    let parsed = parse_wav(&data).unwrap();
    assert_eq!(parsed.pcm.len(), 8);

    let clip = SoundClip::from_wav(data.into()).unwrap();
    let mut out = [0.0f32; 4];
    let mut render = ClipRender::new(prepare_for_device(&clip, device(44_100, 2)), 1.0);
    render.render(&mut out);

    assert_eq!(out[0], 1.0);
    assert_eq!(out[1], -1.0);
    assert_eq!(out[2], 0.0);
    assert!((out[3] - 0.5).abs() < 0.001);
}

#[test]
fn unknown_chunks_before_data_are_skipped_with_padding() {
    let mut data = wav(48_000, 1, &[100, 200]);
    let insert_at = 36;
    let extra = [b'L', b'I', b'S', b'T', 3, 0, 0, 0, 1, 2, 3, 0];
    let _ = data.splice(insert_at..insert_at, extra);

    let parsed = parse_wav(&data).unwrap();
    assert_eq!(parsed.pcm, &[100u8, 0, 200, 0][..]);
}

#[test]
fn a_streaming_size_reads_what_the_file_holds() {
    let mut data = wav(48_000, 1, &[1, 2, 3]);
    let size_at = data.len() - 6 - 4;
    data[size_at..size_at + 4].copy_from_slice(&u32::MAX.to_le_bytes());

    assert_eq!(parse_wav(&data).unwrap().pcm.len(), 6);
}

#[test]
fn a_half_frame_at_the_end_is_dropped() {
    let mut data = wav(48_000, 2, &[1, 2, 3, 4]);
    data.push(9);
    data.push(9);
    let size_at = 40;
    data[size_at..size_at + 4].copy_from_slice(&10u32.to_le_bytes());

    assert_eq!(parse_wav(&data).unwrap().pcm.len(), 8);
}

#[test]
fn malformed_and_unsupported_files_are_refused() {
    assert!(matches!(parse_wav(b"not a wav"), Err(SoundClipError::Malformed(_))));

    let mut float = wav(48_000, 1, &[0, 0]);
    float[20..22].copy_from_slice(&3u16.to_le_bytes());
    assert_eq!(
        parse_wav(&float),
        Err(SoundClipError::UnsupportedEncoding {
            format_tag: 3,
            bits_per_sample: 16
        })
    );

    let surround = wav(48_000, 6, &[0; 12]);
    assert_eq!(parse_wav(&surround), Err(SoundClipError::UnsupportedChannels(6)));

    let slow = wav(1_000, 1, &[0, 0]);
    assert_eq!(parse_wav(&slow), Err(SoundClipError::UnsupportedSampleRate(1_000)));

    let empty = wav(48_000, 1, &[]);
    assert_eq!(parse_wav(&empty), Err(SoundClipError::Empty));

    let mut no_data = wav(48_000, 1, &[0]);
    no_data.truncate(36);
    assert_eq!(parse_wav(&no_data), Err(SoundClipError::Malformed("no data chunk")));
}

#[test]
fn volume_scales_every_sample_and_is_clamped() {
    let samples: Arc<[f32]> = vec![1.0, -0.5].into();

    let mut out = [0.0f32; 2];
    ClipRender::new(samples.clone(), 0.78).render(&mut out);
    assert!((out[0] - 0.78).abs() < 1e-6);
    assert!((out[1] + 0.39).abs() < 1e-6);

    assert_eq!(clamp_volume(1.5), 1.0);
    assert_eq!(clamp_volume(-0.2), 0.0);
    assert_eq!(clamp_volume(f32::NAN), 0.0);
}

#[test]
fn the_end_is_reported_one_block_after_the_last_sample() {
    let samples: Arc<[f32]> = vec![0.25; 6].into();
    let mut render = ClipRender::new(samples, 1.0);
    let mut out = [9.0f32; 4];

    assert_eq!(render.render(&mut out), RenderState::Playing);
    assert_eq!(out, [0.25; 4]);

    assert_eq!(render.render(&mut out), RenderState::Draining);
    assert_eq!(out, [0.25, 0.25, 0.0, 0.0]);

    assert_eq!(render.render(&mut out), RenderState::Finished);
    assert_eq!(out, [0.0; 4]);
    assert_eq!(render.render(&mut out), RenderState::Finished);
}

#[test]
fn a_clip_ending_on_a_block_boundary_still_drains_first() {
    let mut render = ClipRender::new(vec![0.1; 4].into(), 1.0);
    let mut out = [0.0f32; 4];

    assert_eq!(render.render(&mut out), RenderState::Draining);
    assert_eq!(render.render(&mut out), RenderState::Finished);
}

#[test]
fn mono_fills_every_device_channel_and_stereo_folds_to_mono() {
    assert_eq!(remix(&[0.5, -0.5], 1, 2), vec![0.5, 0.5, -0.5, -0.5]);
    assert_eq!(remix(&[0.2, 0.4], 2, 1), vec![0.3]);
    assert_eq!(remix(&[0.2, 0.4], 2, 4), vec![0.2, 0.4, 0.0, 0.0]);
    assert_eq!(remix(&[0.2, 0.4], 2, 2), vec![0.2, 0.4]);
}

#[test]
fn a_clip_is_resampled_to_the_device_rate_with_its_length_kept() {
    let samples = vec![1_000i16; 48_000];
    let clip = SoundClip::from_wav(wav(48_000, 1, &samples).into()).unwrap();

    let prepared = prepare_for_device(&clip, device(44_100, 2));
    assert_eq!(prepared.len(), 44_100 * 2);
    let middle = prepared[44_100];
    assert!((middle - 1_000.0 / i16::MAX as f32).abs() < 1e-4);
}

#[test]
fn only_a_missing_device_counts_as_unsupported() {
    assert!(SoundClipError::NoOutputDevice.is_unsupported());
    assert!(SoundClipError::NoUsableConfiguration.is_unsupported());
    assert!(!SoundClipError::Empty.is_unsupported());
    assert!(!SoundClipError::DeviceFailed("busy".to_owned()).is_unsupported());
}

#[test]
fn asking_for_an_output_device_never_panics() {
    let _ = playback_support();
}
