use super::audio_capture::{AacQuality, AacSettings, AudioCaptureError};

pub const MF_AAC_SAMPLE_RATES: [u32; 2] = [44_100, 48_000];
pub const MF_AAC_BYTES_PER_SECOND: [u32; 4] = [12_000, 16_000, 20_000, 24_000];
pub const HNS_PER_SECOND: u64 = 10_000_000;
pub const AAC_FRAME_LENGTH: usize = 1024;
const PCM_BYTES_PER_SAMPLE: u32 = 2;

/// the two media types of the sink writer stream, pcm in and aac out with same rate and channels
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct MfAacFormat {
    pub take_rate: u32,
    pub sample_rate: u32,
    pub channels: u32,
    pub aac_bytes_per_second: u32,
    pub pcm_block_alignment: u32,
    pub pcm_bytes_per_second: u32,
}

impl MfAacFormat {
    pub fn for_settings(settings: &AacSettings) -> Result<Self, AudioCaptureError> {
        settings.validate()?;
        let sample_rate = media_foundation_rate(settings.sample_rate);
        let channels = u32::from(settings.channels);
        let pcm_block_alignment = channels * PCM_BYTES_PER_SAMPLE;
        Ok(Self {
            take_rate: settings.sample_rate,
            sample_rate,
            channels,
            aac_bytes_per_second: aac_bytes_per_second(settings),
            pcm_block_alignment,
            pcm_bytes_per_second: sample_rate * pcm_block_alignment,
        })
    }

    /// true if the writer resamples the take before the encoder sees it
    pub const fn resamples(&self) -> bool {
        self.take_rate != self.sample_rate
    }
}

/// encoder rate for an aac-lc take rate, the take rate if the encoder accepts it
/// else the lowest accepted rate above it, eg 22.05 kHz is written at 44.1 kHz
pub fn media_foundation_rate(take_rate: u32) -> u32 {
    MF_AAC_SAMPLE_RATES
        .iter()
        .copied()
        .find(|rate| *rate >= take_rate)
        .unwrap_or(MF_AAC_SAMPLE_RATES[MF_AAC_SAMPLE_RATES.len() - 1])
}

/// encoder rate closest to the take wish that doesnt fall below it
/// windows lowest is 96 kbit/s so a 32 kbit/s voice message goes out at 96
/// same m4a, only bigger
pub fn aac_bytes_per_second(settings: &AacSettings) -> u32 {
    let lowest = MF_AAC_BYTES_PER_SECOND[0];
    let highest = MF_AAC_BYTES_PER_SECOND[MF_AAC_BYTES_PER_SECOND.len() - 1];
    match settings.bit_rate {
        Some(bits_per_second) => {
            let wanted = bits_per_second.div_ceil(8);
            MF_AAC_BYTES_PER_SECOND
                .iter()
                .copied()
                .find(|rate| *rate >= wanted)
                .unwrap_or(highest)
        }
        None => match settings.quality {
            AacQuality::Medium => lowest,
            AacQuality::High => MF_AAC_BYTES_PER_SECOND[1],
        },
    }
}

/// appends float pcm as little endian signed 16 bit pcm
/// out of range samples clip and nan becomes silence
pub fn append_pcm16(interleaved: &[f32], out: &mut Vec<u8>) {
    out.reserve(interleaved.len() * PCM_BYTES_PER_SAMPLE as usize);
    for sample in interleaved {
        let value = if sample.is_nan() {
            0
        } else {
            (sample.clamp(-1.0, 1.0) * f32::from(i16::MAX)).round() as i16
        };
        out.extend_from_slice(&value.to_le_bytes());
    }
}

/// time and duration of each sample in 100 ns units counted from frames already written
/// so rounding never drifts and a sample ends exactly where the next starts
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct SampleClock {
    sample_rate: u32,
    frames: u64,
}

impl SampleClock {
    pub const fn new(sample_rate: u32) -> Self {
        Self { sample_rate, frames: 0 }
    }

    pub const fn frames(&self) -> u64 {
        self.frames
    }

    /// returns (time, duration) of a sample of that many frames then counts them as written
    pub fn advance(&mut self, frames: u64) -> (i64, i64) {
        let start = self.position(self.frames);
        self.frames += frames;
        let end = self.position(self.frames);
        (start, end - start)
    }

    fn position(&self, frames: u64) -> i64 {
        let units = u128::from(frames) * u128::from(HNS_PER_SECOND) / u128::from(self.sample_rate);
        i64::try_from(units).unwrap_or(i64::MAX)
    }
}

/// interleaved silence of one aac frame, written when a take stops before any sound arrived
/// so finalize leaves a playable file instead of failing
pub fn silent_aac_frame(channels: u32) -> Vec<f32> {
    vec![0.0; AAC_FRAME_LENGTH * channels as usize]
}
