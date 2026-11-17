use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::thread;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use rise_platform::PlatformSupport;
use thiserror::Error;

use crate::audio::cpal_sink::supported_configs;
use crate::audio::format::{FormatError, SampleFormat, StreamFormat};
use crate::audio::output::{self, DeviceConfig, OutputError};
use crate::audio::resample::Resampler;
use crate::audio::sound_mute::silence_if_muted;

pub const PCM_BITS_PER_SAMPLE: u16 = 16;
pub const MAXIMUM_CLIP_CHANNELS: u16 = 2;

const WAVE_FORMAT_PCM: u16 = 0x0001;
const WAVE_FORMAT_EXTENSIBLE: u16 = 0xFFFE;
const RESAMPLER_FLUSH_FRAMES: usize = 4;
const EVENT_CAPACITY: usize = 4;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum SoundClipError {
    #[error("the sound is not a readable WAV file: {0}")]
    Malformed(&'static str),
    #[error("the WAV encoding (format {format_tag}, {bits_per_sample} bits) is not 16-bit PCM")]
    UnsupportedEncoding { format_tag: u16, bits_per_sample: u16 },
    #[error("the WAV has {0} channels; only mono and stereo are played")]
    UnsupportedChannels(u16),
    #[error("the WAV sample rate {0} Hz is outside what the player accepts")]
    UnsupportedSampleRate(u32),
    #[error("the WAV holds no audio")]
    Empty,
    #[error("no audio output device is connected")]
    NoOutputDevice,
    #[error("the output device offers no configuration the player can use")]
    NoUsableConfiguration,
    #[error("the output device could not be opened: {0}")]
    DeviceFailed(String),
    #[error("the playback thread could not start: {0}")]
    ThreadFailed(String),
}

impl SoundClipError {
    /// host has no way to play the sound at all, not just this clip being bad
    pub fn is_unsupported(&self) -> bool {
        matches!(self, Self::NoOutputDevice | Self::NoUsableConfiguration)
    }
}

impl From<OutputError> for SoundClipError {
    fn from(error: OutputError) -> Self {
        match error {
            OutputError::NoDevice => Self::NoOutputDevice,
            OutputError::NoUsableConfiguration => Self::NoUsableConfiguration,
            OutputError::DeviceFailed => Self::DeviceFailed("the device refused the stream".to_owned()),
        }
    }
}

impl From<FormatError> for SoundClipError {
    fn from(error: FormatError) -> Self {
        match error {
            FormatError::UnsupportedSampleRate { sample_rate } => Self::UnsupportedSampleRate(sample_rate),
            FormatError::UnsupportedChannelCount { channels } => Self::UnsupportedChannels(channels),
        }
    }
}

pub fn playback_support() -> PlatformSupport {
    match cpal::default_host().default_output_device() {
        Some(_) => PlatformSupport::Performed,
        None => PlatformSupport::Unsupported,
    }
}

/// decoded clip, interleaved float samples in the file own rate and channel count
#[derive(Clone, Debug)]
pub struct SoundClip {
    format: StreamFormat,
    samples: Arc<[f32]>,
}

impl SoundClip {
    /// parses a RIFF wave file of 16 bit little endian pcm, mono or stereo
    pub fn from_wav(data: Arc<[u8]>) -> Result<SoundClip, SoundClipError> {
        let wav = parse_wav(&data)?;
        let samples: Arc<[f32]> = wav
            .pcm
            .chunks_exact(2)
            .map(|pair| i16::from_le_bytes([pair[0], pair[1]]) as f32 / i16::MAX as f32)
            .collect();

        Ok(SoundClip {
            format: wav.format,
            samples,
        })
    }

    pub fn format(&self) -> StreamFormat {
        self.format
    }

    pub fn frames(&self) -> u64 {
        self.format.frames(self.samples.len() as u64)
    }

    pub fn duration_millis(&self) -> u64 {
        self.format.frames_to_millis(self.frames())
    }

    /// starts the clip on the default output at volume 0.0 to 1.0, err means nothing started
    /// on_finish(true) runs on the playback thread after the last sample, false if the stream fails
    /// never called after stop or drop
    pub fn play(
        &self,
        volume: f32,
        on_finish: impl FnOnce(bool) + Send + 'static,
    ) -> Result<SoundClipPlayback, SoundClipError> {
        let (events, inbox) = mpsc::sync_channel::<ClipEvent>(EVENT_CAPACITY);
        let (started_sender, started) = mpsc::sync_channel::<Result<(), SoundClipError>>(1);
        let stopped = Arc::new(AtomicBool::new(false));

        let clip = self.clone();
        let callback_events = events.clone();
        let thread_stopped = stopped.clone();
        let volume = clamp_volume(volume);

        thread::Builder::new()
            .name("rise-sound-clip".to_owned())
            .spawn(move || {
                let stream = match open_stream(&clip, volume, callback_events) {
                    Ok(stream) => stream,
                    Err(error) => {
                        let _ = started_sender.send(Err(error));
                        return;
                    }
                };
                let _ = started_sender.send(Ok(()));
                run_until_done(stream, inbox, &thread_stopped, on_finish);
            })
            .map_err(|error| SoundClipError::ThreadFailed(error.to_string()))?;

        match started.recv() {
            Ok(Ok(())) => Ok(SoundClipPlayback { stopped, events }),
            Ok(Err(error)) => Err(error),
            Err(_) => Err(SoundClipError::ThreadFailed(
                "the playback thread ended before the stream started".to_owned(),
            )),
        }
    }
}

/// a clip that is playing, stopping or dropping it silences the device without calling on_finish
pub struct SoundClipPlayback {
    stopped: Arc<AtomicBool>,
    events: SyncSender<ClipEvent>,
}

impl SoundClipPlayback {
    /// stops playback, the thread drops the stream and on_finish is not called
    pub fn stop(&self) {
        if self.stopped.swap(true, Ordering::AcqRel) {
            return;
        }
        let _ = self.events.try_send(ClipEvent::Stop);
    }

    pub fn is_stopped(&self) -> bool {
        self.stopped.load(Ordering::Acquire)
    }
}

impl Drop for SoundClipPlayback {
    fn drop(&mut self) {
        self.stop();
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum ClipEvent {
    Finished,
    Failed,
    Stop,
}

/// what one device block left behind
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum RenderState {
    Playing,
    Draining,
    Finished,
}

/// device side of a clip, already in the device rate and channel count so a block is a copy
pub(crate) struct ClipRender {
    samples: Arc<[f32]>,
    cursor: usize,
    volume: f32,
    drained: bool,
}

impl ClipRender {
    pub(crate) fn new(samples: Arc<[f32]>, volume: f32) -> Self {
        Self {
            samples,
            cursor: 0,
            volume: clamp_volume(volume),
            drained: false,
        }
    }

    pub(crate) fn render(&mut self, out: &mut [f32]) -> RenderState {
        let available = self.samples.len() - self.cursor;
        let copied = available.min(out.len());

        for (target, source) in out[..copied]
            .iter_mut()
            .zip(&self.samples[self.cursor..self.cursor + copied])
        {
            *target = source * self.volume;
        }
        out[copied..].fill(0.0);
        self.cursor += copied;

        if copied > 0 {
            return if self.cursor < self.samples.len() {
                RenderState::Playing
            } else {
                self.drained = true;
                RenderState::Draining
            };
        }
        if self.drained || self.samples.is_empty() {
            RenderState::Finished
        } else {
            RenderState::Playing
        }
    }
}

/// volume range 0.0 silent to 1.0 full, anything else clamped and NaN is silent
pub(crate) fn clamp_volume(volume: f32) -> f32 {
    if volume.is_nan() {
        0.0
    } else {
        volume.clamp(0.0, 1.0)
    }
}

/// parts of a wav file the clip reads
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ParsedWav<'a> {
    pub(crate) format: StreamFormat,
    pub(crate) pcm: &'a [u8],
}

pub(crate) fn parse_wav(data: &[u8]) -> Result<ParsedWav<'_>, SoundClipError> {
    if data.len() < 12 || &data[0..4] != b"RIFF" || &data[8..12] != b"WAVE" {
        return Err(SoundClipError::Malformed("no RIFF WAVE header"));
    }

    let mut format: Option<StreamFormat> = None;
    let mut position = 12usize;

    while position + 8 <= data.len() {
        let id = &data[position..position + 4];
        let declared = read_u32(data, position + 4) as usize;
        let body_start = position + 8;
        let body_end = body_start.saturating_add(declared);

        match id {
            b"fmt " => {
                if body_end > data.len() {
                    return Err(SoundClipError::Malformed("the fmt chunk is cut off"));
                }
                format = Some(parse_format(&data[body_start..body_end])?);
            }
            b"data" => {
                let format = format.ok_or(SoundClipError::Malformed("the data chunk comes before fmt"))?;
                let end = body_end.min(data.len());
                let frame_bytes = format.channels as usize * (PCM_BITS_PER_SAMPLE as usize / 8);
                let whole = (end - body_start) / frame_bytes * frame_bytes;
                if whole == 0 {
                    return Err(SoundClipError::Empty);
                }
                return Ok(ParsedWav {
                    format,
                    pcm: &data[body_start..body_start + whole],
                });
            }
            _ => {}
        }

        position = body_end.saturating_add(declared & 1);
    }

    Err(SoundClipError::Malformed("no data chunk"))
}

fn parse_format(chunk: &[u8]) -> Result<StreamFormat, SoundClipError> {
    if chunk.len() < 16 {
        return Err(SoundClipError::Malformed("the fmt chunk is shorter than 16 bytes"));
    }

    let mut format_tag = read_u16(chunk, 0);
    let channels = read_u16(chunk, 2);
    let sample_rate = read_u32(chunk, 4);
    let block_align = read_u16(chunk, 12);
    let bits_per_sample = read_u16(chunk, 14);

    if format_tag == WAVE_FORMAT_EXTENSIBLE {
        if chunk.len() < 40 {
            return Err(SoundClipError::Malformed("the extensible fmt chunk is cut off"));
        }
        format_tag = read_u16(chunk, 24);
    }

    if format_tag != WAVE_FORMAT_PCM || bits_per_sample != PCM_BITS_PER_SAMPLE {
        return Err(SoundClipError::UnsupportedEncoding {
            format_tag,
            bits_per_sample,
        });
    }
    if channels == 0 || channels > MAXIMUM_CLIP_CHANNELS {
        return Err(SoundClipError::UnsupportedChannels(channels));
    }
    if block_align != channels * (PCM_BITS_PER_SAMPLE / 8) {
        return Err(SoundClipError::Malformed("the block alignment does not match the channels"));
    }

    Ok(StreamFormat::new(sample_rate, channels)?)
}

fn read_u16(data: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([data[at], data[at + 1]])
}

fn read_u32(data: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([data[at], data[at + 1], data[at + 2], data[at + 3]])
}

/// converts the clip to what the device opened, rate first then channel layout
pub(crate) fn prepare_for_device(clip: &SoundClip, device: DeviceConfig) -> Arc<[f32]> {
    let source_channels = clip.format.channels as usize;

    let resampled: Vec<f32> = if device.sample_rate == clip.format.sample_rate {
        clip.samples.to_vec()
    } else {
        let mut resampler = Resampler::new(clip.format, device.sample_rate);
        let mut out = Vec::new();
        resampler.process(&clip.samples, &mut out);
        resampler.process(&vec![0.0; RESAMPLER_FLUSH_FRAMES * source_channels], &mut out);
        let expected = (clip.samples.len() / source_channels) as u64 * device.sample_rate as u64
            / clip.format.sample_rate as u64;
        out.truncate(expected as usize * source_channels);
        out
    };

    remix(&resampled, source_channels, device.channels as usize).into()
}

/// mono goes to every device channel
/// stereo keeps left and right and leaves extra channels silent, stereo on a mono device is averaged
pub(crate) fn remix(samples: &[f32], source_channels: usize, device_channels: usize) -> Vec<f32> {
    if source_channels == device_channels || source_channels == 0 || device_channels == 0 {
        return samples.to_vec();
    }

    let frames = samples.len() / source_channels;
    let mut out = Vec::with_capacity(frames * device_channels);

    for frame in samples.chunks_exact(source_channels) {
        match (source_channels, device_channels) {
            (1, _) => out.extend(std::iter::repeat_n(frame[0], device_channels)),
            (_, 1) => out.push(frame.iter().sum::<f32>() / source_channels as f32),
            _ => {
                for channel in 0..device_channels {
                    out.push(frame.get(channel).copied().unwrap_or(0.0));
                }
            }
        }
    }

    out
}

fn open_stream(
    clip: &SoundClip,
    volume: f32,
    events: SyncSender<ClipEvent>,
) -> Result<cpal::Stream, SoundClipError> {
    let host = cpal::default_host();
    let device = host.default_output_device().ok_or(SoundClipError::NoOutputDevice)?;
    let supported = supported_configs(&device)?;
    let config = output::choose(clip.format, &supported)?;

    let mut render = ClipRender::new(prepare_for_device(clip, config), volume);
    let stream_config = cpal::StreamConfig {
        channels: config.channels,
        sample_rate: cpal::SampleRate(config.sample_rate),
        buffer_size: cpal::BufferSize::Default,
    };

    let error_events = events.clone();
    let on_error = move |error: cpal::StreamError| {
        tracing::error!(?error, "sound clip output stream failed");
        let _ = error_events.try_send(ClipEvent::Failed);
    };

    let mut reported = false;
    let mut block = move |out: &mut [f32]| {
        let state = render.render(out);
        silence_if_muted(out);
        if state == RenderState::Finished && !reported {
            reported = true;
            let _ = events.try_send(ClipEvent::Finished);
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
    .map_err(|error| SoundClipError::DeviceFailed(error.to_string()))?;

    stream
        .play()
        .map_err(|error| SoundClipError::DeviceFailed(error.to_string()))?;

    Ok(stream)
}

fn run_until_done(
    stream: cpal::Stream,
    inbox: Receiver<ClipEvent>,
    stopped: &AtomicBool,
    on_finish: impl FnOnce(bool),
) {
    let outcome = match inbox.recv() {
        Ok(ClipEvent::Finished) => Some(true),
        Ok(ClipEvent::Failed) => Some(false),
        Ok(ClipEvent::Stop) | Err(_) => None,
    };
    drop(stream);

    if let Some(successfully) = outcome
        && !stopped.load(Ordering::Acquire)
    {
        on_finish(successfully);
    }
}
