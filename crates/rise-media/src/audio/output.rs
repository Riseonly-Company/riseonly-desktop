//! Choosing what to ask the sound device for.
//!
//! WASAPI shared mode does not negotiate — it is fixed at the rate set in the
//! sound control panel, so a file at another rate must be resampled by us.

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
    /// True when the device would not take the file's rate. The pump reads this
    /// rather than comparing rates itself.
    pub resampling: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum OutputError {
    NoDevice,
    NoUsableConfiguration,
    DeviceFailed,
}

/// How much audio is buffered ahead of the device.
pub const BUFFER_MILLIS: u64 = 300;

pub const fn ring_samples(config: DeviceConfig) -> usize {
    (config.sample_rate as u64 * BUFFER_MILLIS / 1000 * config.channels as u64) as usize
}

/// Pick a configuration for this file on this device.
///
/// Preference order: the file's own rate, then channel count, then sample format.
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
                // Fewer channels than the file downmixes and loses one; more is free.
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

/// f32 needs no conversion, i32 keeps a 24-bit master intact, i16 is lossy.
const fn format_rank(format: SampleFormat) -> u8 {
    match format {
        SampleFormat::F32 => 0,
        SampleFormat::I32 => 1,
        SampleFormat::I16 => 2,
    }
}

/// A sound device that plays what the ring holds.
///
/// Deliberately not `Send`: `cpal::Stream` is not `Send` on every backend, so
/// the sink stays on the thread that opened it.
pub trait AudioSink {
    fn config(&self) -> DeviceConfig;
    fn play(&mut self) -> Result<(), OutputError>;
    fn pause(&mut self) -> Result<(), OutputError>;
    /// Frames the device has actually consumed. Playback position comes from
    /// this, never from wall-clock time — the device clock drifts.
    fn frames_played(&self) -> u64;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cd() -> StreamFormat {
        StreamFormat::new(44_100, 2).unwrap()
    }

    fn coreaudio() -> Vec<SupportedConfig> {
        vec![SupportedConfig {
            channels: 2,
            minimum_sample_rate: 8_000,
            maximum_sample_rate: 96_000,
            sample_format: SampleFormat::F32,
        }]
    }

    fn wasapi_shared() -> Vec<SupportedConfig> {
        vec![SupportedConfig {
            channels: 2,
            minimum_sample_rate: 48_000,
            maximum_sample_rate: 48_000,
            sample_format: SampleFormat::F32,
        }]
    }

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
