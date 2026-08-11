//! Decode, trim, resample, remap, apply gain, hand to the device.
//!
//! Order matters: resampling precedes the channel map because the resampler
//! keeps per-channel history, and gain comes last so a duck ramps what is
//! actually heard. Runs on the decode thread, never in the device callback.

use super::decoder::{AudioDecoder, DecodeError, Decoded};
use super::fade::{OnComplete, VolumeRamp};
use super::format::StreamFormat;
use super::gapless::{Trim, Trimmer};
use super::output::DeviceConfig;
use super::resample::Resampler;
use super::ring::Producer;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PumpStep {
    /// Audio was produced and there is room for more.
    Produced(usize),
    /// The ring is full. The decode thread sleeps rather than spinning.
    Full,
    /// The track is finished. Everything decoded is already in the ring.
    EndOfStream,
}

pub struct Pump {
    decoder: Box<dyn AudioDecoder>,
    source: StreamFormat,
    config: DeviceConfig,
    trimmer: Trimmer,
    resampler: Option<Resampler>,
    ramp: VolumeRamp,
    producer: Producer,
    decoded: Vec<f32>,
    converted: Vec<f32>,
    mapped: Vec<f32>,
    frames_produced: u64,
    ended: bool,
}

impl Pump {
    pub fn new(
        decoder: Box<dyn AudioDecoder>,
        trim: Trim,
        config: DeviceConfig,
        producer: Producer,
    ) -> Self {
        let source = decoder.format();
        let resampler = if config.resampling {
            Some(Resampler::new(source, config.sample_rate))
        } else {
            None
        };

        Self {
            decoder,
            source,
            config,
            trimmer: Trimmer::new(trim),
            resampler,
            ramp: VolumeRamp::full(),
            producer,
            decoded: Vec::new(),
            converted: Vec::new(),
            mapped: Vec::new(),
            frames_produced: 0,
            ended: false,
        }
    }

    pub fn ramp(&mut self) -> &mut VolumeRamp {
        &mut self.ramp
    }

    pub fn frames_produced(&self) -> u64 {
        self.frames_produced
    }

    pub fn is_ended(&self) -> bool {
        self.ended
    }

    pub fn buffered_samples(&self) -> usize {
        self.producer.filled()
    }

    /// Millisecond position of the audio most recently pushed into the ring —
    /// ahead of what is heard, which comes from the sink's frame counter.
    pub fn produced_millis(&self) -> u64 {
        self.frames_produced * 1000 / self.config.sample_rate as u64
    }

    /// Decode until the ring is full or the track ends.
    pub fn fill(&mut self) -> Result<PumpStep, DecodeError> {
        if self.ended {
            return Ok(PumpStep::EndOfStream);
        }

        // Wait for half the ring; refilling per free sample is a spin loop.
        if self.producer.free() < self.producer.capacity() / 2 {
            return Ok(PumpStep::Full);
        }

        if self.trimmer.is_complete() {
            self.ended = true;
            return Ok(PumpStep::EndOfStream);
        }

        self.decoded.clear();
        match self.decoder.decode(&mut self.decoded)? {
            Decoded::EndOfStream => {
                self.ended = true;
                return Ok(PumpStep::EndOfStream);
            }
            Decoded::Frames(0) => return Ok(PumpStep::Produced(0)),
            Decoded::Frames(_) => {}
        }

        let source_channels = self.source.channels as usize;
        let kept = self.trimmer.accept(self.decoded.len() / source_channels);
        if kept.is_empty() {
            return Ok(PumpStep::Produced(0));
        }
        let trimmed = &self.decoded[kept.start * source_channels..kept.end * source_channels];

        self.converted.clear();
        let at_device_rate: &[f32] = match self.resampler.as_mut() {
            Some(resampler) => {
                resampler.process(trimmed, &mut self.converted);
                &self.converted
            }
            None => trimmed,
        };

        self.mapped.clear();
        map_channels(
            at_device_rate,
            self.source.channels,
            self.config.channels,
            &mut self.mapped,
        );

        if let Some(OnComplete::Pause) = self.ramp.apply(&mut self.mapped, self.config.channels) {
            // The duck finished; the caller stops the device, the ring is silent.
        }

        let written = self.producer.push(&self.mapped);
        let frames = written / self.config.channels as usize;
        self.frames_produced += frames as u64;

        Ok(PumpStep::Produced(frames))
    }

    /// Move to a new position and discard everything buffered from the old one.
    pub fn seek(&mut self, millis: u64) -> Result<u64, DecodeError> {
        let reached = self.decoder.seek(millis)?;

        self.trimmer.seek_to(self.source.millis_to_frames(reached));
        if let Some(resampler) = self.resampler.as_mut() {
            resampler.reset();
        }
        self.frames_produced = reached * self.config.sample_rate as u64 / 1000;
        self.ended = false;

        Ok(reached)
    }
}

/// Fit `source_channels` of interleaved audio into `device_channels`.
///
/// Mono fans out, everything-to-mono averages, and a wider source is truncated
/// to the leading channels — there is no downmix matrix.
pub fn map_channels(input: &[f32], source_channels: u16, device_channels: u16, out: &mut Vec<f32>) {
    let source = source_channels.max(1) as usize;
    let device = device_channels.max(1) as usize;

    if source == device {
        out.extend_from_slice(input);
        return;
    }

    for frame in input.chunks_exact(source) {
        if source == 1 {
            for _ in 0..device {
                out.push(frame[0]);
            }
            continue;
        }

        if device == 1 {
            let sum: f32 = frame.iter().sum();
            out.push(sum / source as f32);
            continue;
        }

        for channel in 0..device {
            out.push(frame.get(channel).copied().unwrap_or(0.0));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::format::SampleFormat;
    use super::super::ring;
    use super::super::symphonia_backend::{SymphoniaDecoder, tests::wav};
    use super::*;

    fn config(sample_rate: u32, channels: u16, resampling: bool) -> DeviceConfig {
        DeviceConfig {
            sample_rate,
            channels,
            sample_format: SampleFormat::F32,
            resampling,
        }
    }

    fn pump_for(bytes: Vec<u8>, config: DeviceConfig, trim: Trim) -> (Pump, ring::Consumer) {
        let decoder = SymphoniaDecoder::open(bytes).unwrap();
        let (producer, consumer) = ring::ring(1 << 16);
        (
            Pump::new(Box::new(decoder), trim, config, producer),
            consumer,
        )
    }

    /// Asks the ring only for what it holds, so no underrun is manufactured.
    fn drain(pump: &mut Pump, consumer: &mut ring::Consumer) -> Vec<f32> {
        let mut collected = Vec::new();

        let mut running = true;
        while running || consumer.filled() > 0 {
            if running && pump.fill().unwrap() == PumpStep::EndOfStream {
                running = false;
            }

            let available = consumer.filled().min(4096);
            if available == 0 {
                continue;
            }

            let mut block = vec![0.0; available];
            let count = consumer.pop_or_silence(&mut block);
            collected.extend_from_slice(&block[..count]);
        }

        collected
    }

    #[test]
    fn a_track_reaches_the_device_frame_for_frame() {
        let (mut pump, mut consumer) =
            pump_for(wav(44_100, 2, 44_100), config(44_100, 2, false), Trim::NONE);

        let played = drain(&mut pump, &mut consumer);

        assert_eq!(played.len(), 44_100 * 2);
        assert_eq!(pump.frames_produced(), 44_100);
        assert_eq!(consumer.underruns(), 0);
    }

    #[test]
    fn the_priming_frames_never_reach_the_device() {
        let trim = Trim {
            start_frames: 1_000,
            valid_frames: Some(20_000),
        };
        let (mut pump, mut consumer) =
            pump_for(wav(44_100, 2, 44_100), config(44_100, 2, false), trim);

        let played = drain(&mut pump, &mut consumer);

        assert_eq!(
            played.len(),
            20_000 * 2,
            "the encoder's padding is what the gap between album tracks is made of"
        );
    }

    #[test]
    fn a_forty_four_one_track_on_a_forty_eight_device_comes_out_at_the_device_rate() {
        let (mut pump, mut consumer) =
            pump_for(wav(44_100, 2, 44_100), config(48_000, 2, true), Trim::NONE);

        let played = drain(&mut pump, &mut consumer);
        let frames = played.len() / 2;

        assert!(
            (frames as i64 - 48_000).abs() < 200,
            "one second of audio came out as {frames} frames at 48 kHz"
        );
    }

    #[test]
    fn a_mono_file_plays_out_of_both_speakers() {
        let (mut pump, mut consumer) =
            pump_for(wav(44_100, 1, 4_410), config(44_100, 2, false), Trim::NONE);

        let played = drain(&mut pump, &mut consumer);

        assert_eq!(played.len(), 4_410 * 2);
        for frame in played.chunks(2) {
            assert_eq!(frame[0], frame[1]);
        }
    }

    #[test]
    fn a_duck_reaches_the_device_as_a_ramp_and_not_a_cut() {
        let (mut pump, mut consumer) =
            pump_for(wav(44_100, 2, 44_100), config(44_100, 2, false), Trim::NONE);
        pump.ramp().duck_and_pause(4_410);

        let played = drain(&mut pump, &mut consumer);

        let tail = &played[played.len() - 1000..];
        assert!(
            tail.iter().all(|sample| sample.abs() < 1e-6),
            "the duck did not reach silence"
        );
        let head = &played[..64];
        assert!(
            head.iter().any(|sample| sample.abs() > 0.01),
            "the duck cut instead of ramping"
        );
    }

    #[test]
    fn seeking_moves_the_position_and_keeps_playing() {
        let (mut pump, _consumer) = pump_for(
            wav(44_100, 2, 44_100 * 5),
            config(44_100, 2, false),
            Trim::NONE,
        );
        pump.fill().unwrap();

        let reached = pump.seek(2_000).unwrap();

        assert!(reached.abs_diff(2_000) < 50);
        assert!(pump.produced_millis().abs_diff(2_000) < 50);
        assert!(matches!(pump.fill().unwrap(), PumpStep::Produced(_)));
    }

    #[test]
    fn the_decode_thread_stops_when_the_ring_is_full_rather_than_spinning() {
        let decoder = SymphoniaDecoder::open(wav(44_100, 2, 44_100 * 5)).unwrap();
        let (producer, _consumer) = ring::ring(4096);
        let mut pump = Pump::new(
            Box::new(decoder),
            Trim::NONE,
            config(44_100, 2, false),
            producer,
        );

        let mut saw_full = false;
        for _ in 0..100 {
            if pump.fill().unwrap() == PumpStep::Full {
                saw_full = true;
                break;
            }
        }

        assert!(saw_full);
    }

    #[test]
    fn stereo_into_six_channels_leaves_the_extra_ones_silent() {
        let mut out = Vec::new();

        map_channels(&[1.0, -1.0, 0.5, -0.5], 2, 6, &mut out);

        assert_eq!(
            out,
            vec![1.0, -1.0, 0.0, 0.0, 0.0, 0.0, 0.5, -0.5, 0.0, 0.0, 0.0, 0.0]
        );
    }

    #[test]
    fn six_channels_into_stereo_takes_the_front_pair() {
        let mut out = Vec::new();

        map_channels(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0], 6, 2, &mut out);

        assert_eq!(out, vec![1.0, 2.0]);
    }

    #[test]
    fn everything_into_mono_is_the_average() {
        let mut out = Vec::new();

        map_channels(&[1.0, 0.0], 2, 1, &mut out);

        assert_eq!(out, vec![0.5]);
    }

    #[test]
    fn matching_channel_counts_are_copied_untouched() {
        let mut out = Vec::new();
        let input = vec![1.0, 2.0, 3.0, 4.0];

        map_channels(&input, 2, 2, &mut out);

        assert_eq!(out, input);
    }
}
