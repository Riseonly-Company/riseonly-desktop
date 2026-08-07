//! Sample rates, channel counts and the arithmetic between frames and time.
//!
//! Small and dull on purpose. Every position bar, every seek, every gapless
//! join and every buffer size is this arithmetic, and a rounding error here is
//! a track that ends a few milliseconds early or a scrubber that drifts over
//! an hour-long DJ set.
//!
//! Pure: no device, no decoder, no OS.

/// One decoded frame is one sample per channel. Positions are counted in
/// frames, never in samples — the difference is a factor of two that silently
/// halves or doubles every duration on stereo.
pub type Frames = u64;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum SampleFormat {
    /// What the pipeline works in, and what every backend can accept.
    F32,
    I16,
    I32,
}

impl SampleFormat {
    pub const fn bytes_per_sample(self) -> usize {
        match self {
            Self::I16 => 2,
            Self::F32 | Self::I32 => 4,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FormatError {
    UnsupportedSampleRate { sample_rate: u32 },
    UnsupportedChannelCount { channels: u16 },
}

/// The narrowest and widest a file is allowed to claim.
///
/// A container is attacker-supplied. Without a bound, a header claiming
/// 4 000 000 000 Hz sizes every buffer in the pipeline off that number.
pub const MINIMUM_SAMPLE_RATE: u32 = 4_000;
pub const MAXIMUM_SAMPLE_RATE: u32 = 384_000;
pub const MAXIMUM_CHANNELS: u16 = 8;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct StreamFormat {
    pub sample_rate: u32,
    pub channels: u16,
}

impl StreamFormat {
    pub fn new(sample_rate: u32, channels: u16) -> Result<Self, FormatError> {
        if !(MINIMUM_SAMPLE_RATE..=MAXIMUM_SAMPLE_RATE).contains(&sample_rate) {
            return Err(FormatError::UnsupportedSampleRate { sample_rate });
        }
        if channels == 0 || channels > MAXIMUM_CHANNELS {
            return Err(FormatError::UnsupportedChannelCount { channels });
        }

        Ok(Self {
            sample_rate,
            channels,
        })
    }

    pub const fn samples(&self, frames: Frames) -> u64 {
        frames * self.channels as u64
    }

    pub const fn frames(&self, samples: u64) -> Frames {
        samples / self.channels as u64
    }

    /// Truncating, because a position that rounds up reads as one millisecond
    /// past a track that has not finished.
    pub const fn frames_to_millis(&self, frames: Frames) -> u64 {
        frames * 1000 / self.sample_rate as u64
    }

    /// Rounding to nearest, because a seek target that always truncates walks
    /// backwards a fraction of a frame every time the user scrubs.
    pub const fn millis_to_frames(&self, millis: u64) -> Frames {
        (millis * self.sample_rate as u64 + 500) / 1000
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cd() -> StreamFormat {
        StreamFormat::new(44_100, 2).unwrap()
    }

    #[test]
    fn a_header_claiming_an_impossible_rate_is_refused_before_anything_is_sized() {
        assert_eq!(
            StreamFormat::new(0, 2),
            Err(FormatError::UnsupportedSampleRate { sample_rate: 0 })
        );
        assert_eq!(
            StreamFormat::new(4_000_000_000, 2),
            Err(FormatError::UnsupportedSampleRate {
                sample_rate: 4_000_000_000
            })
        );
        assert_eq!(
            StreamFormat::new(44_100, 0),
            Err(FormatError::UnsupportedChannelCount { channels: 0 })
        );
        assert_eq!(
            StreamFormat::new(44_100, 64),
            Err(FormatError::UnsupportedChannelCount { channels: 64 })
        );
    }

    #[test]
    fn every_rate_the_catalogue_actually_uses_is_accepted() {
        for rate in [8_000, 22_050, 44_100, 48_000, 88_200, 96_000, 192_000] {
            assert!(StreamFormat::new(rate, 2).is_ok(), "{rate} Hz");
        }
    }

    #[test]
    fn frames_are_not_samples() {
        assert_eq!(cd().samples(1_000), 2_000);
        assert_eq!(cd().frames(2_000), 1_000);
    }

    #[test]
    fn a_three_minute_track_reports_three_minutes() {
        let frames = 44_100 * 180;
        assert_eq!(cd().frames_to_millis(frames), 180_000);
    }

    #[test]
    fn seeking_to_a_position_and_reading_it_back_does_not_drift() {
        let format = cd();
        let mut millis = 0;

        for _ in 0..10_000 {
            millis += 37;
            let frames = format.millis_to_frames(millis);
            let read_back = format.frames_to_millis(frames);
            assert!(
                read_back.abs_diff(millis) <= 1,
                "at {millis} ms the position read back as {read_back} ms"
            );
        }
    }

    #[test]
    fn an_hour_long_set_does_not_overflow_or_lose_precision() {
        let format = StreamFormat::new(48_000, 2).unwrap();
        let frames = 48_000 * 3_600 * 3;

        assert_eq!(format.frames_to_millis(frames), 10_800_000);
        assert_eq!(format.millis_to_frames(10_800_000), frames);
    }
}
