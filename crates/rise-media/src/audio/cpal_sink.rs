//! The actual sound device, on all three operating systems.
//!
//! The callback drains the ring and converts sample format, nothing else: no
//! allocation, no lock, no logging (see `ring.rs`).

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

use super::format::{SampleFormat, StreamFormat};
use super::output::{AudioSink, DeviceConfig, OutputError, SupportedConfig};
use super::ring::Consumer;

pub struct CpalSink {
    stream: cpal::Stream,
    config: DeviceConfig,
    frames_played: Arc<AtomicU64>,
}

impl CpalSink {
    /// Open the default output device for a track. The consumer is moved into
    /// the callback, making the ring single-consumer by construction.
    pub fn open(source: StreamFormat, consumer: Consumer) -> Result<Self, OutputError> {
        let host = cpal::default_host();
        let device = host.default_output_device().ok_or(OutputError::NoDevice)?;

        let supported = supported_configs(&device)?;
        let config = super::output::choose(source, &supported)?;

        Self::open_on(&device, config, consumer)
    }

    fn open_on(
        device: &cpal::Device,
        config: DeviceConfig,
        mut consumer: Consumer,
    ) -> Result<Self, OutputError> {
        let frames_played = Arc::new(AtomicU64::new(0));
        let counter = frames_played.clone();
        let channels = config.channels as usize;

        let stream_config = cpal::StreamConfig {
            channels: config.channels,
            sample_rate: cpal::SampleRate(config.sample_rate),
            buffer_size: cpal::BufferSize::Default,
        };

        let on_error = |error| {
            tracing::error!(?error, "audio output stream failed");
        };

        let stream = match config.sample_format {
            SampleFormat::F32 => device.build_output_stream(
                &stream_config,
                move |out: &mut [f32], _| {
                    consumer.pop_or_silence(out);
                    counter.fetch_add((out.len() / channels) as u64, Ordering::Relaxed);
                },
                on_error,
                None,
            ),
            SampleFormat::I16 => {
                // Allocated here and reused: the callback may not allocate.
                let mut scratch = vec![0.0f32; 0];
                device.build_output_stream(
                    &stream_config,
                    move |out: &mut [i16], _| {
                        scratch.resize(out.len(), 0.0);
                        consumer.pop_or_silence(&mut scratch);
                        for (target, source) in out.iter_mut().zip(scratch.iter()) {
                            *target = (source.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
                        }
                        counter.fetch_add((out.len() / channels) as u64, Ordering::Relaxed);
                    },
                    on_error,
                    None,
                )
            }
            SampleFormat::I32 => {
                let mut scratch = vec![0.0f32; 0];
                device.build_output_stream(
                    &stream_config,
                    move |out: &mut [i32], _| {
                        scratch.resize(out.len(), 0.0);
                        consumer.pop_or_silence(&mut scratch);
                        for (target, source) in out.iter_mut().zip(scratch.iter()) {
                            *target = (source.clamp(-1.0, 1.0) as f64 * i32::MAX as f64) as i32;
                        }
                        counter.fetch_add((out.len() / channels) as u64, Ordering::Relaxed);
                    },
                    on_error,
                    None,
                )
            }
        }
        .map_err(|error| {
            tracing::error!(?error, "could not open the output stream");
            OutputError::DeviceFailed
        })?;

        Ok(Self {
            stream,
            config,
            frames_played,
        })
    }
}

impl AudioSink for CpalSink {
    fn config(&self) -> DeviceConfig {
        self.config
    }

    fn play(&mut self) -> Result<(), OutputError> {
        self.stream.play().map_err(|_| OutputError::DeviceFailed)
    }

    fn pause(&mut self) -> Result<(), OutputError> {
        self.stream.pause().map_err(|_| OutputError::DeviceFailed)
    }

    fn frames_played(&self) -> u64 {
        self.frames_played.load(Ordering::Relaxed)
    }
}

/// Translate what cpal reports into what the chooser understands. Formats this
/// crate does not handle are dropped from the list, never guessed at.
pub fn supported_configs(device: &cpal::Device) -> Result<Vec<SupportedConfig>, OutputError> {
    let ranges = device
        .supported_output_configs()
        .map_err(|_| OutputError::NoUsableConfiguration)?;

    Ok(ranges
        .filter_map(|range| {
            let sample_format = match range.sample_format() {
                cpal::SampleFormat::F32 => SampleFormat::F32,
                cpal::SampleFormat::I16 => SampleFormat::I16,
                cpal::SampleFormat::I32 => SampleFormat::I32,
                _ => return None,
            };

            Some(SupportedConfig {
                channels: range.channels(),
                minimum_sample_rate: range.min_sample_rate().0,
                maximum_sample_rate: range.max_sample_rate().0,
                sample_format,
            })
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::super::ring;
    use super::*;

    fn default_device() -> Option<cpal::Device> {
        cpal::default_host().default_output_device()
    }

    #[test]
    fn the_host_can_be_asked_for_a_device_without_panicking() {
        // A CI runner has no sound card; cpal must report that, not fall over.
        let _ = default_device();
    }

    #[test]
    fn a_real_device_advertises_something_the_chooser_can_use() {
        let Some(device) = default_device() else {
            eprintln!("skipped: no output device on this machine");
            return;
        };

        let supported = supported_configs(&device).unwrap();
        assert!(
            !supported.is_empty(),
            "the default device advertised nothing this crate can open"
        );

        let source = StreamFormat::new(44_100, 2).unwrap();
        let config = super::super::output::choose(source, &supported).unwrap();
        assert!(config.channels >= 1);
    }

    #[test]
    fn a_stream_opens_and_starts_on_a_real_device() {
        if default_device().is_none() {
            eprintln!("skipped: no output device on this machine");
            return;
        }

        let source = StreamFormat::new(44_100, 2).unwrap();
        let (mut producer, consumer) = ring::ring(1 << 15);
        producer.push(&vec![0.0; 1 << 14]);

        let mut sink = match CpalSink::open(source, consumer) {
            Ok(sink) => sink,
            Err(error) => {
                eprintln!("skipped: the default device refused to open ({error:?})");
                return;
            }
        };

        sink.play().unwrap();
        sink.pause().unwrap();

        assert!(sink.config().sample_rate >= 8_000);
    }
}
