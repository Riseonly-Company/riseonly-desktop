//! Choosing what to ask the sound device for.
//!
//! The three operating systems disagree about what a device is willing to do,
//! and the disagreement is not cosmetic:
//!
//!   - CoreAudio will usually open at the file's own rate, so nothing is
//!     converted and nothing is lost.
//!   - WASAPI in shared mode is FIXED at whatever the user set in the sound
//!     control panel. It does not negotiate. A 44.1 kHz track played on a
//!     48 kHz device has to be resampled by us.
//!   - ALSA and PulseAudio advertise ranges, and what they will actually open
//!     depends on the plugin chain behind them.
//!
//! So the choice is a pure function of what the device advertises and what the
//! file is, and all three shapes are tested on a Mac. The one thing this
//! refuses to do is silently accept a rate that is not the file's without
//! saying so: `DeviceConfig::resampling` is what tells the pump to insert the
//! converter, and the alternative to knowing is playing the catalogue sharp.

use super::format::{SampleFormat, StreamFormat};

/// One configuration a device says it can open.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct SupportedConfig {
    pub channels: u16,
    pub minimum_sample_rate: u32,
    pub maximum_sample_rate: u32,
    pub sample_format: SampleFormat,
}

impl SupportedConfig {
    pub const fn supports(&self, sample_rate: u32) -> bool {
        self.minimum_sample_rate <= sample_rate && sample_rate <= self.maximum_sample_rate
    }

    const fn nearest(&self, sample_rate: u32) -> u32 {
        if sample_rate < self.minimum_sample_rate {
            self.minimum_sample_rate
        } else if sample_rate > self.maximum_sample_rate {
            self.maximum_sample_rate
        } else {
            sample_rate
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct DeviceConfig {
    pub sample_rate: u32,
    pub channels: u16,
    pub sample_format: SampleFormat,
    /// True when the device would not take the file's rate. The pump reads
    /// this; nothing else may infer it by comparing numbers itself.
    pub resampling: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum OutputError {
    NoDevice,
    NoUsableConfiguration,
    DeviceFailed,
}

/// How much audio is buffered ahead of the device.
///
/// 300 ms. Large enough that a decode thread descheduled behind a disk read
/// does not underrun, small enough that a pause or a track change is not
/// visibly late. A desktop has no reason for the tighter budget a phone call
/// path needs, and the reference's player has no low-latency mode at all.
pub const BUFFER_MILLIS: u64 = 300;

pub const fn ring_samples(config: DeviceConfig) -> usize {
    (config.sample_rate as u64 * BUFFER_MILLIS / 1000 * config.channels as u64) as usize
}

/// Pick a configuration for this file on this device.
///
/// Ordered by what it costs to be wrong: playing at the wrong rate is the worst
/// (audible pitch shift), converting sample formats is the cheapest.
pub fn choose(
    source: StreamFormat,
    supported: &[SupportedConfig],
) -> Result<DeviceConfig, OutputError> {
    if supported.is_empty() {
        return Err(OutputError::NoUsableConfiguration);
    }

    let native: Vec<&SupportedConfig> = supported
        .iter()
        .filter(|config| config.supports(source.sample_rate))
        .collect();

    let (pool, sample_rate, resampling) = if native.is_empty() {
        let fallback = supported
            .iter()
            .min_by_key(|config| {
                config
                    .nearest(source.sample_rate)
                    .abs_diff(source.sample_rate)
            })
            .ok_or(OutputError::NoUsableConfiguration)?;
        let rate = fallback.nearest(source.sample_rate);

        let pool: Vec<&SupportedConfig> = supported
            .iter()
            .filter(|config| config.supports(rate))
            .collect();
        (pool, rate, true)
    } else {
        (native, source.sample_rate, false)
    };

    let chosen = pool
        .into_iter()
        .min_by_key(|config| {
            (
                // A device with fewer channels than the file downmixes, which
                // loses a channel outright; more is free.
                config.channels < source.channels,
                config.channels.abs_diff(source.channels),
                format_rank(config.sample_format),
            )
        })
        .ok_or(OutputError::NoUsableConfiguration)?;

    Ok(DeviceConfig {
        sample_rate,
        channels: chosen.channels,
        sample_format: chosen.sample_format,
        resampling,
    })
}

/// f32 is what the pipeline already holds, so it needs no conversion at all.
/// i32 keeps every bit of a 24-bit master; i16 is the lossy one.
const fn format_rank(format: SampleFormat) -> u8 {
    match format {
        SampleFormat::F32 => 0,
        SampleFormat::I32 => 1,
        SampleFormat::I16 => 2,
    }
}

/// A sound device that plays what the ring holds.
///
/// The trait exists so the pump, the clock and every policy above are tested
/// with no device at all — CI runners have no sound card, and a test suite that
/// needs one is a test suite that does not run.
///
/// Deliberately not `Send`. `cpal::Stream` is not `Send` on every backend, so
/// the sink stays on the thread that opened it and the ring is the only thing
/// that crosses threads. That is the correct shape anyway: the thing that
/// crosses is audio, not ownership of a device.
pub trait AudioSink {
    fn config(&self) -> DeviceConfig;
    fn play(&mut self) -> Result<(), OutputError>;
    fn pause(&mut self) -> Result<(), OutputError>;
    /// Frames the device has actually consumed. The playback position comes
    /// from this and never from wall-clock time: they diverge as soon as the
    /// device's clock drifts, which it does.
    fn frames_played(&self) -> u64;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cd() -> StreamFormat {
        StreamFormat::new(44_100, 2).unwrap()
    }

    /// What a CoreAudio device typically advertises: a wide range, f32.
    fn coreaudio() -> Vec<SupportedConfig> {
        vec![SupportedConfig {
            channels: 2,
            minimum_sample_rate: 8_000,
            maximum_sample_rate: 96_000,
            sample_format: SampleFormat::F32,
        }]
    }

    /// What WASAPI shared mode advertises: exactly one rate, the one the user
    /// picked in the control panel.
    fn wasapi_shared() -> Vec<SupportedConfig> {
        vec![SupportedConfig {
            channels: 2,
            minimum_sample_rate: 48_000,
            maximum_sample_rate: 48_000,
            sample_format: SampleFormat::F32,
        }]
    }

    /// A typical ALSA device: several formats, a couple of channel counts.
    fn alsa() -> Vec<SupportedConfig> {
        vec![
            SupportedConfig {
                channels: 2,
                minimum_sample_rate: 44_100,
                maximum_sample_rate: 192_000,
                sample_format: SampleFormat::I16,
            },
            SupportedConfig {
                channels: 2,
                minimum_sample_rate: 44_100,
                maximum_sample_rate: 192_000,
                sample_format: SampleFormat::F32,
            },
            SupportedConfig {
                channels: 6,
                minimum_sample_rate: 44_100,
                maximum_sample_rate: 192_000,
                sample_format: SampleFormat::F32,
            },
        ]
    }

    #[test]
    fn a_device_that_takes_the_files_rate_is_never_resampled() {
        let config = choose(cd(), &coreaudio()).unwrap();

        assert_eq!(config.sample_rate, 44_100);
        assert!(!config.resampling);
    }

    #[test]
    fn a_fixed_rate_device_is_recognised_as_needing_conversion() {
        let config = choose(cd(), &wasapi_shared()).unwrap();

        assert_eq!(config.sample_rate, 48_000);
        assert!(
            config.resampling,
            "not saying so is how the whole catalogue ends up playing sharp"
        );
    }

    #[test]
    fn a_forty_eight_kilohertz_file_on_the_same_device_needs_nothing() {
        let source = StreamFormat::new(48_000, 2).unwrap();

        let config = choose(source, &wasapi_shared()).unwrap();

        assert!(!config.resampling);
    }

    #[test]
    fn float_output_is_preferred_over_a_conversion() {
        let config = choose(cd(), &alsa()).unwrap();

        assert_eq!(config.sample_format, SampleFormat::F32);
        assert_eq!(config.channels, 2);
    }

    #[test]
    fn a_stereo_file_does_not_open_a_surround_configuration() {
        let config = choose(cd(), &alsa()).unwrap();

        assert_eq!(config.channels, 2);
    }

    #[test]
    fn a_mono_file_plays_through_the_stereo_device_rather_than_failing() {
        let source = StreamFormat::new(44_100, 1).unwrap();

        let config = choose(source, &coreaudio()).unwrap();

        assert_eq!(config.channels, 2);
    }

    #[test]
    fn a_device_with_no_configurations_is_an_error_not_a_guess() {
        assert_eq!(choose(cd(), &[]), Err(OutputError::NoUsableConfiguration));
    }

    #[test]
    fn a_rate_below_everything_the_device_offers_lands_on_its_minimum() {
        let source = StreamFormat::new(8_000, 1).unwrap();
        let supported = vec![SupportedConfig {
            channels: 2,
            minimum_sample_rate: 44_100,
            maximum_sample_rate: 48_000,
            sample_format: SampleFormat::F32,
        }];

        let config = choose(source, &supported).unwrap();

        assert_eq!(config.sample_rate, 44_100);
        assert!(config.resampling);
    }

    #[test]
    fn the_ring_holds_about_three_hundred_milliseconds() {
        let config = choose(cd(), &coreaudio()).unwrap();

        let samples = ring_samples(config);

        assert_eq!(samples, 44_100 * 300 / 1000 * 2);
    }
}
