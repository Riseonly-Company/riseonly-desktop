use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::thread;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use thiserror::Error;

use crate::audio::cpal_sink::supported_configs;
use crate::audio::decoder::{AudioDecoder, DecodeError, Decoded};
use crate::audio::format::{SampleFormat, StreamFormat};
use crate::audio::gapless::Trimmer;
use crate::audio::output::{self, DeviceConfig};
use crate::audio::resample::Resampler;
use crate::audio::sound_mute::silence_if_muted;
use crate::audio::symphonia_backend::SymphoniaDecoder;
use crate::sound_clip::sound_clip::{clamp_volume, remix};

const RESAMPLER_FLUSH_FRAMES: usize = 4;
const EVENT_CAPACITY: usize = 4;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum AVAudioPlayerError {
    #[error("the sound could not be decoded: {0:?}")]
    Decode(DecodeError),
    #[error("the sound holds no audio")]
    Empty,
}

impl From<DecodeError> for AVAudioPlayerError {
    fn from(error: DecodeError) -> Self {
        Self::Decode(error)
    }
}

/// sound decoded from memory, ready to play on the default output device
pub struct AVAudioPlayer {
    format: StreamFormat,
    samples: Arc<[f32]>,
    number_of_loops: i32,
    volume: Arc<AtomicU32>,
    position: Arc<AtomicUsize>,
    prepared: Option<PreparedSound>,
    playback: Option<Playback>,
}

/// the sound converted to the config the device opens with
struct PreparedSound {
    config: DeviceConfig,
    samples: Arc<[f32]>,
}

/// one running stream, dropping it stops the stream thread
struct Playback {
    active: Arc<AtomicBool>,
    events: SyncSender<PlayerEvent>,
}

impl Playback {
    fn stop(&self) {
        if !self.active.swap(false, Ordering::AcqRel) {
            return;
        }
        let _ = self.events.try_send(PlayerEvent::Stop);
    }

    fn is_active(&self) -> bool {
        self.active.load(Ordering::Acquire)
    }
}

impl Drop for Playback {
    fn drop(&mut self) {
        self.stop();
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum PlayerEvent {
    Finished,
    Failed,
    Stop,
}

impl AVAudioPlayer {
    /// decodes every frame once, any container the crate reads
    pub fn with_data(data: Vec<u8>) -> Result<Self, AVAudioPlayerError> {
        let mut decoder = SymphoniaDecoder::open(data)?;
        let format = decoder.format();
        let channels = format.channels as usize;
        let mut trimmer = Trimmer::new(SymphoniaDecoder::trim(&decoder));
        let mut samples = Vec::<f32>::new();
        let mut block = Vec::<f32>::new();

        loop {
            block.clear();
            match decoder.decode(&mut block)? {
                Decoded::EndOfStream => break,
                Decoded::Frames(_) => {
                    let kept = trimmer.accept(block.len() / channels.max(1));
                    if !kept.is_empty() {
                        samples.extend_from_slice(&block[kept.start * channels..kept.end * channels]);
                    }
                    if trimmer.is_complete() {
                        break;
                    }
                }
            }
        }

        if samples.len() < channels.max(1) {
            return Err(AVAudioPlayerError::Empty);
        }

        Ok(Self {
            format,
            samples: samples.into(),
            number_of_loops: 0,
            volume: Arc::new(AtomicU32::new(1.0_f32.to_bits())),
            position: Arc::new(AtomicUsize::new(0)),
            prepared: None,
            playback: None,
        })
    }

    /// 0 plays once and n plays n + 1 times, any negative repeats until stop
    pub fn set_number_of_loops(&mut self, loops: i32) {
        self.number_of_loops = loops;
    }

    pub fn number_of_loops(&self) -> i32 {
        self.number_of_loops
    }

    /// volume 0.0 to 1.0, applied in the device callback so it changes a sound that is already playing
    pub fn set_volume(&mut self, volume: f32) {
        self.volume.store(clamp_volume(volume).to_bits(), Ordering::Relaxed);
    }

    pub fn volume(&self) -> f32 {
        f32::from_bits(self.volume.load(Ordering::Relaxed))
    }

    pub fn is_playing(&self) -> bool {
        self.playback.as_ref().is_some_and(Playback::is_active)
    }

    /// picks the device config and converts the sound so play only opens the stream
    /// false when the host has no output
    pub fn prepare_to_play(&mut self) -> bool {
        if self.prepared.is_some() {
            return true;
        }
        let Some(device) = cpal::default_host().default_output_device() else {
            tracing::warn!("no audio output device for the player");
            return false;
        };
        let config = match supported_configs(&device).and_then(|supported| output::choose(self.format, &supported)) {
            Ok(config) => config,
            Err(error) => {
                tracing::warn!(?error, "the audio output offers no usable configuration");
                return false;
            }
        };

        self.prepared = Some(PreparedSound {
            config,
            samples: prepare_for_device(self.format, &self.samples, config),
        });
        true
    }

    /// starts or resumes, true once the device stream is running
    pub fn play(&mut self) -> bool {
        if self.is_playing() {
            return true;
        }
        if !self.prepare_to_play() {
            return false;
        }
        let Some(prepared) = self.prepared.as_ref() else {
            return false;
        };

        let (events, inbox) = mpsc::sync_channel::<PlayerEvent>(EVENT_CAPACITY);
        let (started_sender, started) = mpsc::sync_channel::<bool>(1);
        let active = Arc::new(AtomicBool::new(true));

        let start = self.position.load(Ordering::Relaxed);
        let render = LoopRender::new(
            prepared.samples.clone(),
            start,
            remaining_loops(self.number_of_loops),
            self.volume.clone(),
            self.position.clone(),
        );
        let config = prepared.config;
        let callback_events = events.clone();
        let thread_active = active.clone();
        let thread_position = self.position.clone();

        let spawned = thread::Builder::new()
            .name("rise-av-audio-player".to_owned())
            .spawn(move || {
                let stream = match open_stream(config, render, callback_events) {
                    Ok(stream) => stream,
                    Err(error) => {
                        tracing::warn!(%error, "the audio player could not open the output stream");
                        thread_active.store(false, Ordering::Release);
                        let _ = started_sender.send(false);
                        return;
                    }
                };
                let _ = started_sender.send(true);
                run_until_done(stream, inbox, &thread_active, &thread_position);
            });
        if let Err(error) = spawned {
            tracing::warn!(%error, "the audio player thread could not start");
            return false;
        }

        if started.recv().unwrap_or(false) {
            self.playback = Some(Playback { active, events });
            true
        } else {
            false
        }
    }

    /// silences the device, position is kept so play resumes there
    pub fn stop(&mut self) {
        if let Some(playback) = self.playback.take() {
            playback.stop();
        }
    }
}

impl Drop for AVAudioPlayer {
    fn drop(&mut self) {
        self.stop();
    }
}

/// how many more times the sound restarts after this pass, none repeats forever
fn remaining_loops(number_of_loops: i32) -> Option<u32> {
    u32::try_from(number_of_loops).ok()
}

/// what one device block left behind
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum RenderState {
    Playing,
    Draining,
    Finished,
}

/// device side of the player, samples in the device format and a cursor that wraps while loops remain
/// plus the live volume
struct LoopRender {
    samples: Arc<[f32]>,
    cursor: usize,
    remaining_loops: Option<u32>,
    volume: Arc<AtomicU32>,
    position: Arc<AtomicUsize>,
    drained: bool,
}

impl LoopRender {
    fn new(
        samples: Arc<[f32]>,
        start: usize,
        remaining_loops: Option<u32>,
        volume: Arc<AtomicU32>,
        position: Arc<AtomicUsize>,
    ) -> Self {
        let cursor = if start < samples.len() { start } else { 0 };
        Self {
            samples,
            cursor,
            remaining_loops,
            volume,
            position,
            drained: false,
        }
    }

    fn render(&mut self, out: &mut [f32]) -> RenderState {
        if self.samples.is_empty() {
            out.fill(0.0);
            return RenderState::Finished;
        }

        let volume = f32::from_bits(self.volume.load(Ordering::Relaxed));
        let mut written = 0usize;

        while written < out.len() {
            if self.cursor >= self.samples.len() {
                match self.remaining_loops {
                    Some(0) => break,
                    Some(remaining) => self.remaining_loops = Some(remaining - 1),
                    None => {}
                }
                self.cursor = 0;
            }
            let count = (self.samples.len() - self.cursor).min(out.len() - written);
            for (target, source) in out[written..written + count]
                .iter_mut()
                .zip(&self.samples[self.cursor..self.cursor + count])
            {
                *target = source * volume;
            }
            written += count;
            self.cursor += count;
        }
        out[written..].fill(0.0);
        self.position.store(self.cursor, Ordering::Relaxed);

        if written == out.len() && !(self.cursor >= self.samples.len() && self.remaining_loops == Some(0)) {
            return RenderState::Playing;
        }
        if written > 0 {
            self.drained = true;
            return RenderState::Draining;
        }
        if self.drained {
            RenderState::Finished
        } else {
            self.drained = true;
            RenderState::Draining
        }
    }
}

/// converts the decoded sound to what the device opened, rate first then channel layout
fn prepare_for_device(format: StreamFormat, samples: &[f32], device: DeviceConfig) -> Arc<[f32]> {
    let source_channels = format.channels as usize;

    let resampled: Vec<f32> = if device.sample_rate == format.sample_rate {
        samples.to_vec()
    } else {
        let mut resampler = Resampler::new(format, device.sample_rate);
        let mut out = Vec::new();
        resampler.process(samples, &mut out);
        resampler.process(&vec![0.0; RESAMPLER_FLUSH_FRAMES * source_channels], &mut out);
        let expected =
            (samples.len() / source_channels) as u64 * device.sample_rate as u64 / format.sample_rate as u64;
        out.truncate(expected as usize * source_channels);
        out
    };

    remix(&resampled, source_channels, device.channels as usize).into()
}

fn open_stream(
    config: DeviceConfig,
    mut render: LoopRender,
    events: SyncSender<PlayerEvent>,
) -> Result<cpal::Stream, String> {
    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .ok_or_else(|| "no audio output device is connected".to_owned())?;

    let stream_config = cpal::StreamConfig {
        channels: config.channels,
        sample_rate: cpal::SampleRate(config.sample_rate),
        buffer_size: cpal::BufferSize::Default,
    };

    let error_events = events.clone();
    let on_error = move |error: cpal::StreamError| {
        tracing::error!(?error, "audio player output stream failed");
        let _ = error_events.try_send(PlayerEvent::Failed);
    };

    let mut reported = false;
    let mut block = move |out: &mut [f32]| {
        let state = render.render(out);
        silence_if_muted(out);
        if state == RenderState::Finished && !reported {
            reported = true;
            let _ = events.try_send(PlayerEvent::Finished);
        }
    };

    let stream = match config.sample_format {
        SampleFormat::F32 => device.build_output_stream(
            &stream_config,
            move |out: &mut [f32], _| block(out),
            on_error,
            None,
        ),
        SampleFormat::I16 => {
            let mut scratch = Vec::<f32>::new();
            device.build_output_stream(
                &stream_config,
                move |out: &mut [i16], _| {
                    scratch.resize(out.len(), 0.0);
                    block(&mut scratch);
                    for (target, source) in out.iter_mut().zip(scratch.iter()) {
                        *target = (source.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
                    }
                },
                on_error,
                None,
            )
        }
        SampleFormat::I32 => {
            let mut scratch = Vec::<f32>::new();
            device.build_output_stream(
                &stream_config,
                move |out: &mut [i32], _| {
                    scratch.resize(out.len(), 0.0);
                    block(&mut scratch);
                    for (target, source) in out.iter_mut().zip(scratch.iter()) {
                        *target = (source.clamp(-1.0, 1.0) as f64 * i32::MAX as f64) as i32;
                    }
                },
                on_error,
                None,
            )
        }
    }
    .map_err(|error| error.to_string())?;

    stream.play().map_err(|error| error.to_string())?;

    Ok(stream)
}

fn run_until_done(stream: cpal::Stream, inbox: Receiver<PlayerEvent>, active: &AtomicBool, position: &AtomicUsize) {
    let finished = matches!(inbox.recv(), Ok(PlayerEvent::Finished));
    drop(stream);

    if finished && active.swap(false, Ordering::AcqRel) {
        position.store(0, Ordering::Relaxed);
    }
    active.store(false, Ordering::Release);
}
