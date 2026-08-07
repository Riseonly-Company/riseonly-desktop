//! Playing a 44.1 kHz file on a device that only runs at 48 kHz.
//!
//! WHY THIS EXISTS AT ALL
//!
//! On macOS a CoreAudio device will usually open at the file's own rate and
//! nothing here runs. On Windows, WASAPI in shared mode is fixed at whatever
//! the user set in the sound control panel — 48 kHz on almost every machine —
//! and a 44.1 kHz MP3 played through it verbatim comes out 8.8% sharp and 8.8%
//! fast. That is not a subtle artefact; it is the whole catalogue in the wrong
//! key.
//!
//! WHAT THIS IS AND IS NOT
//!
//! Catmull-Rom interpolation over four points. It is well behaved for the
//! ratios that actually occur (44.1↔48, 22.05→48) and it is a hundred lines
//! with no dependency. It is NOT a mastering-grade sample-rate converter: a
//! windowed-sinc polyphase filter has lower aliasing in the top octave. If a
//! measurement ever shows that mattering, `rubato` is the replacement and this
//! file is the seam it drops into.
//!
//! Pure and deterministic, so it is tested on a Mac even though the platform
//! that needs it is the one nobody here can run.

use super::format::StreamFormat;

/// Frames of history kept between blocks.
///
/// Cubic interpolation at position `p` reads `floor(p) - 1` through
/// `floor(p) + 2`, so three input frames have to survive into the next call or
/// every block boundary is a discontinuity — an audible tick at whatever rate
/// the decoder happens to hand over blocks.
const HISTORY_FRAMES: usize = 3;

#[derive(Debug)]
pub struct Resampler {
    channels: usize,
    /// Input frames consumed per output frame.
    step: f64,
    /// Fractional read position within `work`.
    position: f64,
    history: Vec<f32>,
    work: Vec<f32>,
}

impl Resampler {
    pub fn new(source: StreamFormat, device_rate: u32) -> Self {
        let channels = source.channels as usize;

        Self {
            channels,
            step: source.sample_rate as f64 / device_rate as f64,
            position: HISTORY_FRAMES as f64,
            history: vec![0.0; HISTORY_FRAMES * channels],
            work: Vec::new(),
        }
    }

    /// Whether the rates match, in which case the caller skips this entirely
    /// rather than paying for an interpolation that changes nothing.
    pub fn is_identity(&self) -> bool {
        (self.step - 1.0).abs() < f64::EPSILON
    }

    pub fn step(&self) -> f64 {
        self.step
    }

    /// Drop the history. Called on a seek: the three frames from before the
    /// jump would otherwise be interpolated into the first frames after it.
    pub fn reset(&mut self) {
        self.history.fill(0.0);
        self.position = HISTORY_FRAMES as f64;
    }

    /// Resample one interleaved block, appending to `out`.
    pub fn process(&mut self, input: &[f32], out: &mut Vec<f32>) {
        if self.channels == 0 || input.is_empty() {
            return;
        }

        self.work.clear();
        self.work.extend_from_slice(&self.history);
        self.work.extend_from_slice(input);

        let frames = self.work.len() / self.channels;
        if frames <= HISTORY_FRAMES {
            self.keep_history(frames);
            return;
        }

        // The last position whose four-point window is fully inside `work`.
        let limit = (frames - 2) as f64;

        while self.position < limit {
            let index = self.position as usize;
            let fraction = (self.position - index as f64) as f32;

            for channel in 0..self.channels {
                let sample = catmull_rom(
                    self.work[(index - 1) * self.channels + channel],
                    self.work[index * self.channels + channel],
                    self.work[(index + 1) * self.channels + channel],
                    self.work[(index + 2) * self.channels + channel],
                    fraction,
                );
                out.push(sample);
            }

            self.position += self.step;
        }

        self.keep_history(frames);
    }

    fn keep_history(&mut self, frames: usize) {
        let kept = frames.min(HISTORY_FRAMES);
        let tail = (frames - kept) * self.channels;

        self.history.clear();
        self.history.extend_from_slice(&self.work[tail..]);
        self.history.resize(HISTORY_FRAMES * self.channels, 0.0);

        // The read position moves with the window: everything before the kept
        // history has been consumed.
        self.position -= (frames - kept) as f64;
        self.position = self.position.max(1.0);
    }
}

fn catmull_rom(previous: f32, current: f32, next: f32, after: f32, t: f32) -> f32 {
    let a = -0.5 * previous + 1.5 * current - 1.5 * next + 0.5 * after;
    let b = previous - 2.5 * current + 2.0 * next - 0.5 * after;
    let c = -0.5 * previous + 0.5 * next;

    ((a * t + b) * t + c) * t + current
}

#[cfg(test)]
mod tests {
    use super::*;

    fn format(rate: u32, channels: u16) -> StreamFormat {
        StreamFormat::new(rate, channels).unwrap()
    }

    fn sine(rate: u32, hz: f64, frames: usize, channels: usize) -> Vec<f32> {
        let mut samples = Vec::with_capacity(frames * channels);
        for frame in 0..frames {
            let value = (frame as f64 / rate as f64 * hz * std::f64::consts::TAU).sin() as f32;
            for _ in 0..channels {
                samples.push(value);
            }
        }
        samples
    }

    fn zero_crossings(samples: &[f32], channels: usize) -> usize {
        samples
            .chunks(channels)
            .map(|frame| frame[0])
            .collect::<Vec<f32>>()
            .windows(2)
            .filter(|pair| pair[0] <= 0.0 && pair[1] > 0.0)
            .count()
    }

    #[test]
    fn matching_rates_are_recognised_as_needing_no_work() {
        assert!(Resampler::new(format(48_000, 2), 48_000).is_identity());
        assert!(!Resampler::new(format(44_100, 2), 48_000).is_identity());
    }

    #[test]
    fn a_constant_signal_stays_constant() {
        let mut resampler = Resampler::new(format(44_100, 2), 48_000);
        let mut out = Vec::new();

        for _ in 0..20 {
            resampler.process(&vec![0.5; 2048], &mut out);
        }

        assert!(!out.is_empty());
        for sample in out.iter().skip(64) {
            assert!(
                (sample - 0.5).abs() < 1e-3,
                "a DC signal came out as {sample}"
            );
        }
    }

    #[test]
    fn the_output_length_follows_the_rate_ratio() {
        let mut resampler = Resampler::new(format(44_100, 2), 48_000);
        let mut out = Vec::new();

        let input_frames = 44_100;
        for _ in 0..(input_frames / 1024) {
            resampler.process(&vec![0.0; 1024 * 2], &mut out);
        }

        let output_frames = out.len() / 2;
        let expected = (input_frames / 1024 * 1024) as f64 * 48_000.0 / 44_100.0;
        assert!(
            (output_frames as f64 - expected).abs() < 32.0,
            "{output_frames} frames out of {input_frames}, expected about {expected}"
        );
    }

    #[test]
    fn a_thousand_hertz_tone_is_still_a_thousand_hertz_afterwards() {
        let mut resampler = Resampler::new(format(44_100, 2), 48_000);
        let input = sine(44_100, 1_000.0, 44_100, 2);
        let mut out = Vec::new();

        for chunk in input.chunks(2048) {
            resampler.process(chunk, &mut out);
        }

        let cycles = zero_crossings(&out, 2);
        assert!(
            (999..=1001).contains(&cycles),
            "a 1 kHz tone came out at about {cycles} Hz — playing 44.1 through a \
             48 kHz device unconverted is exactly this bug"
        );
    }

    #[test]
    fn a_block_boundary_is_not_a_discontinuity() {
        let mut resampler = Resampler::new(format(44_100, 1), 48_000);
        let input = sine(44_100, 440.0, 4096, 1);
        let mut out = Vec::new();

        // Deliberately uneven blocks: a decoder hands over whatever a packet
        // happened to contain, never a round number.
        let mut offset = 0;
        for size in [37, 1153, 512, 71, 2048, 275] {
            let end = (offset + size).min(input.len());
            resampler.process(&input[offset..end], &mut out);
            offset = end;
        }

        let biggest_step = out
            .windows(2)
            .map(|pair| (pair[1] - pair[0]).abs())
            .fold(0.0f32, f32::max);

        assert!(
            biggest_step < 0.1,
            "a jump of {biggest_step} between neighbouring samples is the tick \
             you hear at every block boundary"
        );
    }

    #[test]
    fn upsampling_and_downsampling_both_work() {
        for (source, device) in [(44_100, 48_000), (48_000, 44_100), (22_050, 48_000)] {
            let mut resampler = Resampler::new(format(source, 2), device);
            let mut out = Vec::new();
            resampler.process(&sine(source, 440.0, 8192, 2), &mut out);

            let expected = 8192.0 * device as f64 / source as f64;
            let produced = (out.len() / 2) as f64;
            assert!(
                (produced - expected).abs() < 16.0,
                "{source} -> {device}: {produced} frames, expected about {expected}"
            );
        }
    }

    #[test]
    fn a_seek_clears_the_history_so_the_old_audio_is_not_blended_in() {
        let mut resampler = Resampler::new(format(44_100, 1), 48_000);
        let mut out = Vec::new();
        resampler.process(&vec![1.0; 1024], &mut out);

        resampler.reset();
        out.clear();
        resampler.process(&vec![0.0; 1024], &mut out);

        assert!(out.iter().all(|sample| sample.abs() < 1e-6));
    }

    #[test]
    fn an_empty_block_is_not_a_panic() {
        let mut resampler = Resampler::new(format(44_100, 2), 48_000);
        let mut out = Vec::new();

        resampler.process(&[], &mut out);

        assert!(out.is_empty());
    }
}
