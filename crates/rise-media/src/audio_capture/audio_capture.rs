use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc;
use std::thread::JoinHandle;
use std::time::Duration;

use cpal::Sample;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use parking_lot::Mutex;
use rise_core::foundation::foundation_date::RiseClock;
use rise_platform::{HostOs, PlatformSupport};
use thiserror::Error;

use crate::audio::format::StreamFormat;
use crate::audio::resample::Resampler;
use crate::audio::ring::{self, Consumer, Producer};

pub const METER_INTERVAL: Duration = Duration::from_millis(50);
pub const METER_FLOOR_DB: f32 = -55.0;
pub const SILENCE_DB: f32 = -160.0;
pub const AAC_SAMPLE_RATES: [u32; 9] = [8_000, 11_025, 12_000, 16_000, 22_050, 24_000, 32_000, 44_100, 48_000];
pub const MINIMUM_BIT_RATE: u32 = 8_000;
pub const MAXIMUM_BIT_RATE: u32 = 320_000;

const RING_SECONDS: usize = 2;
const DRAIN_INTERVAL: Duration = Duration::from_millis(10);

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum AacQuality {
    Medium,
    High,
}

impl AacQuality {
    /// raw quality value, same as the AudioConverter quality constant (medium 0x40 and high 0x60)
    pub const fn raw(self) -> u32 {
        match self {
            Self::Medium => 0x40,
            Self::High => 0x60,
        }
    }
}

/// recorder settings of one take
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct AacSettings {
    pub sample_rate: u32,
    pub channels: u16,
    pub bit_rate: Option<u32>,
    pub quality: AacQuality,
}

impl AacSettings {
    /// voice message, 44.1 kHz mono at 32 kbit/s and medium quality
    pub const fn voice_message() -> Self {
        Self {
            sample_rate: 44_100,
            channels: 1,
            bit_rate: Some(32_000),
            quality: AacQuality::Medium,
        }
    }

    /// sound of a video note, 44.1 kHz mono and high quality and no fixed rate
    pub const fn video_note() -> Self {
        Self {
            sample_rate: 44_100,
            channels: 1,
            bit_rate: None,
            quality: AacQuality::High,
        }
    }

    pub fn validate(&self) -> Result<(), AudioCaptureError> {
        if !AAC_SAMPLE_RATES.contains(&self.sample_rate) {
            return Err(AudioCaptureError::InvalidSettings("the sample rate is not an AAC-LC rate"));
        }
        if !(1..=2).contains(&self.channels) {
            return Err(AudioCaptureError::InvalidSettings("a take is mono or stereo"));
        }
        if let Some(bit_rate) = self.bit_rate
            && !(MINIMUM_BIT_RATE..=MAXIMUM_BIT_RATE).contains(&bit_rate)
        {
            return Err(AudioCaptureError::InvalidSettings("the bit rate is outside 8..320 kbit/s"));
        }
        Ok(())
    }

    /// how long that many frames play at the take rate
    pub fn duration_of(&self, frames: u64) -> Duration {
        Duration::from_secs_f64(frames as f64 / f64::from(self.sample_rate))
    }
}

/// 0 to 1 bar height for a level in db
pub fn normalized_power(db: f32) -> f32 {
    let clamped = db.max(METER_FLOOR_DB);
    ((clamped - METER_FLOOR_DB) / -METER_FLOOR_DB).clamp(0.0, 1.0)
}

/// average power since the last read in dBFS
#[derive(Clone, Debug, Default)]
pub struct PowerMeter {
    sum_squares: f64,
    samples: u64,
}

impl PowerMeter {
    pub fn add(&mut self, samples: &[f32]) {
        for sample in samples {
            let value = f64::from(*sample);
            self.sum_squares += value * value;
        }
        self.samples += samples.len() as u64;
    }

    /// level of everything added since the last call, nothing added (paused take) is silence
    pub fn take_average_db(&mut self) -> f32 {
        let samples = std::mem::take(&mut self.samples);
        let sum_squares = std::mem::take(&mut self.sum_squares);
        if samples == 0 {
            return SILENCE_DB;
        }
        let rms = (sum_squares / samples as f64).sqrt();
        if rms <= 0.0 {
            return SILENCE_DB;
        }
        ((20.0 * rms.log10()) as f32).clamp(SILENCE_DB, 0.0)
    }
}

/// mixes interleaved frames from one channel count to another and appends to out
/// mono is the average of all and stereo copies a mono mic to both sides
pub fn remix(input: &[f32], from: u16, to: u16, out: &mut Vec<f32>) {
    let from = usize::from(from);
    let to = usize::from(to);
    if from == 0 || to == 0 {
        return;
    }
    for frame in input.chunks_exact(from) {
        if to == 1 {
            let sum: f32 = frame.iter().sum();
            out.push(sum / from as f32);
        } else {
            for channel in 0..to {
                out.push(frame[channel.min(from - 1)]);
            }
        }
    }
}

#[derive(Clone, PartialEq, Eq, Debug, Error)]
pub enum AudioCaptureError {
    #[error("audio capture is not written for {0:?} yet")]
    BranchNotWritten(HostOs),
    #[error("no AAC encoder: {0}")]
    EncoderUnavailable(String),
    #[error("no microphone is connected")]
    NoInputDevice,
    #[error("the microphone delivers {0}, which the recorder does not convert")]
    UnsupportedInputFormat(String),
    #[error("the microphone could not be opened: {0}")]
    InputFailed(String),
    #[error("the recording settings are rejected: {0}")]
    InvalidSettings(&'static str),
    #[error("the encoder failed: {0}")]
    EncoderFailed(String),
    #[error("the recording file failed: {0}")]
    FileFailed(String),
    #[error("the recorder has already stopped")]
    Stopped,
}

impl AudioCaptureError {
    /// true if the host cant record at all, the screen hides the mic button instead of failing
    pub fn is_unsupported(&self) -> bool {
        matches!(self, Self::BranchNotWritten(_) | Self::EncoderUnavailable(_))
    }
}

/// why a running take paused itself
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Interruption {
    DeviceLost,
    StreamFailed(String),
    EncoderFailed(String),
}

/// one open aac file, lives on the capture thread only
pub trait AacFileWriter {
    /// interleaved float pcm at the take rate and channel count
    fn write(&mut self, interleaved: &[f32]) -> Result<(), AudioCaptureError>;
    /// flushes the encoder and closes the container, the file plays only after this
    fn finish(self: Box<Self>) -> Result<(), AudioCaptureError>;
}

pub trait AacEncoderSource {
    fn support(&self) -> Result<(), AudioCaptureError> {
        Err(AudioCaptureError::BranchNotWritten(HostOs::current()))
    }

    /// creates or truncates path as an m4a file in the take format
    fn create(
        &self,
        _path: &Path,
        _settings: &AacSettings,
    ) -> Result<Box<dyn AacFileWriter>, AudioCaptureError> {
        Err(AudioCaptureError::BranchNotWritten(HostOs::current()))
    }
}

pub fn recording_support() -> PlatformSupport {
    match super::os::NativeAacEncoder.support() {
        Ok(()) => PlatformSupport::Performed,
        Err(_) => PlatformSupport::Unsupported,
    }
}

pub fn recording_unavailable_reason() -> Option<AudioCaptureError> {
    super::os::NativeAacEncoder.support().err()
}

/// opens path as an m4a file the caller fills, eg an offline render
/// settings checked before the os encoder
pub fn aac_file_writer(path: &Path, settings: &AacSettings) -> Result<Box<dyn AacFileWriter>, AudioCaptureError> {
    settings.validate()?;
    super::os::NativeAacEncoder.support()?;
    super::os::NativeAacEncoder.create(path, settings)
}

/// mic frames in and encoded take frames out, mix and resample and meter and write
pub struct CapturePipeline {
    input_channels: u16,
    output_channels: u16,
    resampler: Resampler,
    mixed: Vec<f32>,
    resampled: Vec<f32>,
}

impl CapturePipeline {
    pub fn new(input: StreamFormat, settings: &AacSettings) -> Self {
        let mixed_format = StreamFormat {
            sample_rate: input.sample_rate,
            channels: settings.channels,
        };
        Self {
            input_channels: input.channels,
            output_channels: settings.channels,
            resampler: Resampler::new(mixed_format, settings.sample_rate),
            mixed: Vec::new(),
            resampled: Vec::new(),
        }
    }

    /// feeds one block of mic frames and returns how many take frames it wrote
    pub fn feed(
        &mut self,
        interleaved: &[f32],
        meter: &mut PowerMeter,
        writer: &mut dyn AacFileWriter,
    ) -> Result<u64, AudioCaptureError> {
        self.mixed.clear();
        remix(interleaved, self.input_channels, self.output_channels, &mut self.mixed);
        let block: &[f32] = if self.resampler.is_identity() {
            &self.mixed
        } else {
            self.resampled.clear();
            self.resampler.process(&self.mixed, &mut self.resampled);
            &self.resampled
        };
        if block.is_empty() {
            return Ok(0);
        }
        meter.add(block);
        writer.write(block)?;
        Ok((block.len() / usize::from(self.output_channels)) as u64)
    }
}

#[derive(Default)]
struct Shared {
    paused: AtomicBool,
    input_dead: AtomicBool,
    frames: AtomicU64,
    meter: Mutex<PowerMeter>,
    interruption: Mutex<Option<Interruption>>,
}

impl Shared {
    fn input_failed(&self, error: cpal::StreamError) {
        let reason = match error {
            cpal::StreamError::DeviceNotAvailable => Interruption::DeviceLost,
            cpal::StreamError::BackendSpecific { err } => Interruption::StreamFailed(err.to_string()),
        };
        tracing::warn!(?reason, "microphone stream ended during a take");
        *self.interruption.lock() = Some(reason);
        self.input_dead.store(true, Ordering::Release);
        self.paused.store(true, Ordering::Release);
    }
}

enum InputKeepAlive {
    Device { _stream: cpal::Stream },
    Tester { _signal: TesterSignal },
}

struct Input {
    _keep_alive: InputKeepAlive,
    consumer: Consumer,
    format: StreamFormat,
}

static TESTER_SIGNAL: AtomicBool = AtomicBool::new(false);

pub const TESTER_SIGNAL_FORMAT: StreamFormat = StreamFormat { sample_rate: 48_000, channels: 1 };
pub const TESTER_SIGNAL_HZ: f32 = 440.0;
pub const TESTER_SIGNAL_AMPLITUDE: f32 = 0.1;

pub fn set_tester_signal(on: bool) {
    TESTER_SIGNAL.store(on, Ordering::Release);
}

pub fn uses_tester_signal() -> bool {
    TESTER_SIGNAL.load(Ordering::Acquire)
}

/// one sample of the test tone at frame index
pub fn tester_signal_sample(index: u64) -> f32 {
    let seconds = index as f64 / f64::from(TESTER_SIGNAL_FORMAT.sample_rate);
    let phase = (seconds * f64::from(TESTER_SIGNAL_HZ)).fract();
    (phase * std::f64::consts::TAU).sin() as f32 * TESTER_SIGNAL_AMPLITUDE
}

/// thread that feeds the test tone into the ring at the real rate until dropped
struct TesterSignal {
    stop: Arc<AtomicBool>,
}

impl Drop for TesterSignal {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
    }
}

fn open_tester_input() -> Result<Input, AudioCaptureError> {
    let format = TESTER_SIGNAL_FORMAT;
    let capacity = format.sample_rate as usize * usize::from(format.channels) * RING_SECONDS;
    let (mut producer, consumer) = ring::ring(capacity);
    let stop = Arc::new(AtomicBool::new(false));
    let thread_stop = Arc::clone(&stop);
    std::thread::Builder::new()
        .name("rise-audio-tester-signal".to_owned())
        .spawn(move || {
            let started = RiseClock::monotonic();
            let mut written: u64 = 0;
            let mut block: Vec<f32> = Vec::with_capacity(producer.capacity());
            while !thread_stop.load(Ordering::Acquire) {
                std::thread::sleep(DRAIN_INTERVAL);
                let elapsed = RiseClock::monotonic().saturating_sub(started);
                let due = (elapsed.as_secs_f64() * f64::from(format.sample_rate)) as u64;
                let frames = due.saturating_sub(written).min(producer.free() as u64);
                block.clear();
                block.extend((written..written + frames).map(tester_signal_sample));
                written += producer.push(&block) as u64;
            }
        })
        .map_err(|error| AudioCaptureError::InputFailed(error.to_string()))?;
    Ok(Input {
        _keep_alive: InputKeepAlive::Tester { _signal: TesterSignal { stop } },
        consumer,
        format,
    })
}

fn open_input(shared: &Arc<Shared>) -> Result<Input, AudioCaptureError> {
    if uses_tester_signal() {
        return open_tester_input();
    }
    let host = cpal::default_host();
    let device = host
        .default_input_device()
        .ok_or(AudioCaptureError::NoInputDevice)?;
    let supported = device
        .default_input_config()
        .map_err(|error| AudioCaptureError::InputFailed(error.to_string()))?;
    let config = supported.config();
    let format = StreamFormat {
        sample_rate: config.sample_rate.0,
        channels: config.channels,
    };
    if format.channels == 0 || format.sample_rate == 0 {
        return Err(AudioCaptureError::InputFailed(
            "the microphone reports no channels or no rate".to_owned(),
        ));
    }
    let capacity = format.sample_rate as usize * usize::from(format.channels) * RING_SECONDS;
    let (producer, consumer) = ring::ring(capacity);
    let stream = match supported.sample_format() {
        cpal::SampleFormat::F32 => build_input::<f32>(&device, &config, producer, shared),
        cpal::SampleFormat::I16 => build_input::<i16>(&device, &config, producer, shared),
        cpal::SampleFormat::I32 => build_input::<i32>(&device, &config, producer, shared),
        cpal::SampleFormat::U16 => build_input::<u16>(&device, &config, producer, shared),
        other => return Err(AudioCaptureError::UnsupportedInputFormat(other.to_string())),
    }?;
    stream
        .play()
        .map_err(|error| AudioCaptureError::InputFailed(error.to_string()))?;
    Ok(Input {
        _keep_alive: InputKeepAlive::Device { _stream: stream },
        consumer,
        format,
    })
}

fn build_input<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    mut producer: Producer,
    shared: &Arc<Shared>,
) -> Result<cpal::Stream, AudioCaptureError>
where
    T: cpal::SizedSample + Send + 'static,
    f32: cpal::FromSample<T>,
{
    let channels = usize::from(config.channels);
    let mut scratch: Vec<f32> = Vec::with_capacity(producer.capacity());
    let failed = Arc::clone(shared);
    device
        .build_input_stream(
            config,
            move |data: &[T], _: &cpal::InputCallbackInfo| {
                let whole = (producer.free().min(data.len()) / channels) * channels;
                scratch.clear();
                scratch.extend(data[..whole].iter().map(|sample| (*sample).to_sample::<f32>()));
                producer.push(&scratch);
            },
            move |error| failed.input_failed(error),
            None,
        )
        .map_err(|error| AudioCaptureError::InputFailed(error.to_string()))
}

enum Command {
    Resume(mpsc::SyncSender<Result<(), AudioCaptureError>>),
    Stop(mpsc::SyncSender<Result<Duration, AudioCaptureError>>),
}

struct Worker {
    settings: AacSettings,
    shared: Arc<Shared>,
    input: Option<Input>,
    pipeline: CapturePipeline,
    writer: Box<dyn AacFileWriter>,
    block: Vec<f32>,
    failure: Option<AudioCaptureError>,
}

impl Worker {
    fn drain(&mut self) {
        let Some(input) = self.input.as_mut() else {
            return;
        };
        let channels = usize::from(input.format.channels);
        let ready = (input.consumer.filled() / channels) * channels;
        if ready > 0 {
            self.block.resize(ready, 0.0);
            input.consumer.pop_or_silence(&mut self.block);
            if !self.shared.paused.load(Ordering::Acquire) && self.failure.is_none() {
                let mut meter = self.shared.meter.lock();
                match self.pipeline.feed(&self.block, &mut meter, self.writer.as_mut()) {
                    Ok(frames) => {
                        self.shared.frames.fetch_add(frames, Ordering::Release);
                    }
                    Err(error) => {
                        tracing::error!(%error, "the AAC encoder refused a block");
                        *self.shared.interruption.lock() = Some(Interruption::EncoderFailed(error.to_string()));
                        self.shared.paused.store(true, Ordering::Release);
                        self.failure = Some(error);
                    }
                }
            }
        }
        if self.shared.input_dead.load(Ordering::Acquire) {
            self.input = None;
        }
    }

    fn resume(&mut self) -> Result<(), AudioCaptureError> {
        if let Some(error) = &self.failure {
            return Err(error.clone());
        }
        if self.input.is_none() || self.shared.input_dead.load(Ordering::Acquire) {
            self.input = None;
            self.shared.input_dead.store(false, Ordering::Release);
            let input = open_input(&self.shared)?;
            self.pipeline = CapturePipeline::new(input.format, &self.settings);
            self.input = Some(input);
            *self.shared.interruption.lock() = None;
        }
        self.shared.paused.store(false, Ordering::Release);
        Ok(())
    }

    fn stop(mut self) -> Result<Duration, AudioCaptureError> {
        self.drain();
        self.input = None;
        let frames = self.shared.frames.load(Ordering::Acquire);
        let finished = self.writer.finish();
        if let Some(error) = self.failure {
            return Err(error);
        }
        finished?;
        Ok(self.settings.duration_of(frames))
    }
}

fn run(
    path: PathBuf,
    settings: AacSettings,
    shared: Arc<Shared>,
    commands: mpsc::Receiver<Command>,
    opened: mpsc::SyncSender<Result<(), AudioCaptureError>>,
) {
    let opening = open_input(&shared).and_then(|input| {
        super::os::NativeAacEncoder
            .create(&path, &settings)
            .map(|writer| (input, writer))
    });
    let (input, writer) = match opening {
        Ok(opened_parts) => opened_parts,
        Err(error) => {
            if opened.send(Err(error)).is_err() {
                tracing::debug!("audio capture opener is gone");
            }
            return;
        }
    };
    let mut worker = Worker {
        pipeline: CapturePipeline::new(input.format, &settings),
        settings,
        shared,
        input: Some(input),
        writer,
        block: Vec::new(),
        failure: None,
    };
    if opened.send(Ok(())).is_err() {
        tracing::debug!("audio capture opener is gone");
    }
    loop {
        match commands.recv_timeout(DRAIN_INTERVAL) {
            Ok(Command::Resume(reply)) => {
                let result = worker.resume();
                if reply.send(result).is_err() {
                    tracing::debug!("audio capture resume caller is gone");
                }
            }
            Ok(Command::Stop(reply)) => {
                let result = worker.stop();
                if reply.send(result).is_err() {
                    tracing::debug!("audio capture stop caller is gone");
                }
                return;
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                if let Err(error) = worker.stop() {
                    tracing::warn!(%error, "an abandoned take did not close cleanly");
                }
                return;
            }
        }
        worker.drain();
    }
}

/// a running take, every blocking call (start and resume and stop) opens devices or touches disk
/// so run it on the background executor and never on the main thread
pub struct AudioRecorder {
    path: PathBuf,
    settings: AacSettings,
    shared: Arc<Shared>,
    commands: Option<mpsc::Sender<Command>>,
    worker: Option<JoinHandle<()>>,
}

impl AudioRecorder {
    /// opens the default mic and starts writing path
    /// a live call doesnt block the take, voice and video notes record over a call
    pub fn start(path: impl Into<PathBuf>, settings: AacSettings) -> Result<Self, AudioCaptureError> {
        settings.validate()?;
        super::os::NativeAacEncoder.support()?;
        let path = path.into();
        let shared = Arc::new(Shared::default());
        let (commands, inbox) = mpsc::channel();
        let (opened, open_result) = mpsc::sync_channel(1);
        let worker_shared = Arc::clone(&shared);
        let worker_path = path.clone();
        let worker = std::thread::Builder::new()
            .name("rise-audio-capture".to_owned())
            .spawn(move || run(worker_path, settings, worker_shared, inbox, opened))
            .map_err(|error| AudioCaptureError::InputFailed(error.to_string()))?;
        match open_result.recv() {
            Ok(Ok(())) => Ok(Self {
                path,
                settings,
                shared,
                commands: Some(commands),
                worker: Some(worker),
            }),
            Ok(Err(error)) => {
                join(worker);
                Err(error)
            }
            Err(_) => {
                join(worker);
                Err(AudioCaptureError::InputFailed(
                    "the capture thread ended before the microphone opened".to_owned(),
                ))
            }
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn settings(&self) -> AacSettings {
        self.settings
    }

    /// pauses, the file stays open and resume continues it
    pub fn pause(&self) {
        self.shared.paused.store(true, Ordering::Release);
    }

    /// continues after a pause, after DeviceLost it opens the new default mic
    /// and the take goes on in the same file
    pub fn resume(&self) -> Result<(), AudioCaptureError> {
        let commands = self.commands.as_ref().ok_or(AudioCaptureError::Stopped)?;
        let (reply, answer) = mpsc::sync_channel(1);
        commands
            .send(Command::Resume(reply))
            .map_err(|_| AudioCaptureError::Stopped)?;
        answer.recv().map_err(|_| AudioCaptureError::Stopped)?
    }

    pub fn is_paused(&self) -> bool {
        self.shared.paused.load(Ordering::Acquire)
    }

    /// why the take paused itself, if it did
    pub fn interruption(&self) -> Option<Interruption> {
        self.shared.interruption.lock().clone()
    }

    /// level since the previous read
    pub fn average_power_db(&self) -> f32 {
        self.shared.meter.lock().take_average_db()
    }

    pub fn normalized_level(&self) -> f32 {
        normalized_power(self.average_power_db())
    }

    /// recorded length so far counted in written frames, pauses dont count
    pub fn elapsed(&self) -> Duration {
        self.settings
            .duration_of(self.shared.frames.load(Ordering::Acquire))
    }

    /// stops and finalizes the file and returns its length
    /// the file is readable when this returns
    pub fn stop(mut self) -> Result<Duration, AudioCaptureError> {
        let commands = self.commands.take().ok_or(AudioCaptureError::Stopped)?;
        let (reply, answer) = mpsc::sync_channel(1);
        let sent = commands.send(Command::Stop(reply));
        let result = match sent {
            Ok(()) => answer.recv().map_err(|_| AudioCaptureError::Stopped)?,
            Err(_) => Err(AudioCaptureError::Stopped),
        };
        if let Some(worker) = self.worker.take() {
            join(worker);
        }
        result
    }

    /// stops and deletes the file, what cancel does
    pub fn discard(self) -> Result<(), AudioCaptureError> {
        let path = self.path.clone();
        if let Err(error) = self.stop() {
            tracing::debug!(%error, "a discarded take did not close cleanly");
        }
        match std::fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(AudioCaptureError::FileFailed(error.to_string())),
        }
    }
}

impl Drop for AudioRecorder {
    fn drop(&mut self) {
        if let Some(commands) = self.commands.take() {
            let (reply, _answer) = mpsc::sync_channel(1);
            if commands.send(Command::Stop(reply)).is_err() {
                tracing::debug!("audio capture thread already ended");
            }
        }
    }
}

fn join(worker: JoinHandle<()>) {
    if worker.join().is_err() {
        tracing::error!("the audio capture thread panicked");
    }
}
