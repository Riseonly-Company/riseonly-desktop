use std::collections::VecDeque;
use std::f32::consts::PI;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU8, AtomicU32, Ordering};
use std::sync::mpsc;
use std::thread::JoinHandle;
use std::time::Duration;

use parking_lot::Mutex;
use rise_media::audio::cpal_sink::CpalSink;
use rise_media::audio::decoder::{AudioDecoder, DecodeError, Decoded};
use rise_media::audio::format::StreamFormat;
use rise_media::audio::gapless::{Trim, Trimmer};
use rise_media::audio::output::{AudioSink, OutputError};
use rise_media::audio::pump::map_channels;
use rise_media::audio::resample::Resampler;
use rise_media::audio::ring::{Producer, ring};
use rise_media::audio::symphonia_backend::SymphoniaDecoder;

use crate::modules::music::shared::utils::music::music_audio_effect_settings::{
    AVAudioUnitReverbPreset, MusicAudioEffectDSP, MusicAudioEffectKind, MusicAudioEffectSettings,
};

/// f32 the main thread writes and the render thread reads, like an audio unit parameter
#[derive(Debug, Default)]
struct AtomicF32(AtomicU32);

impl AtomicF32 {
    fn new(value: f32) -> Self {
        Self(AtomicU32::new(value.to_bits()))
    }

    fn load(&self) -> f32 {
        f32::from_bits(self.0.load(Ordering::Relaxed))
    }

    fn store(&self, value: f32) {
        self.0.store(value.to_bits(), Ordering::Relaxed);
    }
}

/// procedural ambience, vinyl crackle rendered in one block
/// synthesised and not sampled so no bundled asset, no licence and no decoding
/// renders identically in the exporter
pub struct MusicAmbienceGenerator {
    seed: u32,
    crackle_envelope: f32,
    crackle_countdown: i64,

    pub vinyl_gain: f32,
    pub is_muted: bool,

    sample_rate: f32,
}

impl MusicAmbienceGenerator {
    pub fn new(sample_rate: f64) -> Self {
        Self {
            seed: 0x9E37_79B9,
            crackle_envelope: 0.0,
            crackle_countdown: 0,
            vinyl_gain: 0.0,
            is_muted: false,
            sample_rate: sample_rate as f32,
        }
    }

    pub fn is_silent(&self) -> bool {
        self.is_muted || self.vinyl_gain <= 0.0001
    }

    pub fn next_sample(&mut self) -> f32 {
        if self.is_silent() {
            return 0.0;
        }
        let white = self.next_white();
        if self.crackle_countdown <= 0 {
            self.crackle_countdown = 220 + (self.next_unit() * self.sample_rate * 0.06) as i64;
            self.crackle_envelope = 0.6 + 0.4 * self.next_unit();
        }
        self.crackle_countdown -= 1;
        self.crackle_envelope *= 0.72;
        let output = (white * 0.06 + white * self.crackle_envelope) * self.vinyl_gain;
        output.clamp(-1.0, 1.0)
    }

    fn next_white(&mut self) -> f32 {
        self.seed = self.seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (self.seed as i32) as f32 / i32::MAX as f32
    }

    fn next_unit(&mut self) -> f32 {
        self.seed = self.seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (self.seed >> 8) as f32 / (1u32 << 24) as f32
    }
}

/// every parameter of every unit in the chain
/// main thread writes them, the render side reads them once per block
#[derive(Debug)]
struct MusicEffectChainParameters {
    varispeed_rate: AtomicF32,
    pitch_cents: AtomicF32,
    pitch_bypass: AtomicBool,
    low_pass_hz: AtomicF32,
    low_pass_bypass: AtomicBool,
    low_shelf_gain_db: AtomicF32,
    low_shelf_bypass: AtomicBool,
    high_pass_hz: AtomicF32,
    high_pass_bypass: AtomicBool,
    global_gain_db: AtomicF32,
    delay_wet_dry_mix: AtomicF32,
    delay_feedback: AtomicF32,
    delay_bypass: AtomicBool,
    reverb_wet_dry_mix: AtomicF32,
    reverb_preset: AtomicU8,
    dynamics_bypass: AtomicBool,
    compression_threshold_db: AtomicF32,
    compression_makeup_db: AtomicF32,
    submix_position_x: AtomicF32,
    submix_position_z: AtomicF32,
    vinyl_gain: AtomicF32,
    ambience_muted: AtomicBool,
}

impl MusicEffectChainParameters {
    fn new() -> Self {
        Self {
            varispeed_rate: AtomicF32::new(1.0),
            pitch_cents: AtomicF32::new(0.0),
            pitch_bypass: AtomicBool::new(true),
            low_pass_hz: AtomicF32::new(20_000.0),
            low_pass_bypass: AtomicBool::new(true),
            low_shelf_gain_db: AtomicF32::new(0.0),
            low_shelf_bypass: AtomicBool::new(true),
            high_pass_hz: AtomicF32::new(20.0),
            high_pass_bypass: AtomicBool::new(true),
            global_gain_db: AtomicF32::new(0.0),
            delay_wet_dry_mix: AtomicF32::new(0.0),
            delay_feedback: AtomicF32::new(0.0),
            delay_bypass: AtomicBool::new(true),
            reverb_wet_dry_mix: AtomicF32::new(0.0),
            reverb_preset: AtomicU8::new(reverb_preset_raw(AVAudioUnitReverbPreset::LargeHall)),
            dynamics_bypass: AtomicBool::new(true),
            compression_threshold_db: AtomicF32::new(0.0),
            compression_makeup_db: AtomicF32::new(0.0),
            submix_position_x: AtomicF32::new(0.0),
            submix_position_z: AtomicF32::new(-MusicEffectAudioChain::ORBIT_RADIUS),
            vinyl_gain: AtomicF32::new(0.0),
            ambience_muted: AtomicBool::new(false),
        }
    }
}

const fn reverb_preset_raw(preset: AVAudioUnitReverbPreset) -> u8 {
    match preset {
        AVAudioUnitReverbPreset::MediumChamber => 0,
        AVAudioUnitReverbPreset::MediumHall => 1,
        AVAudioUnitReverbPreset::LargeHall => 2,
        AVAudioUnitReverbPreset::Cathedral => 3,
    }
}

const fn reverb_preset_from_raw(raw: u8) -> AVAudioUnitReverbPreset {
    match raw {
        0 => AVAudioUnitReverbPreset::MediumChamber,
        1 => AVAudioUnitReverbPreset::MediumHall,
        3 => AVAudioUnitReverbPreset::Cathedral,
        _ => AVAudioUnitReverbPreset::LargeHall,
    }
}

/// builds and tunes the shared effect chain, playback and the exporter both use it so they cant drift
/// node list is fixed whichever effects are on, each unit has a bypass flag
/// 8D is the exception, it needs another connection format
pub struct MusicEffectAudioChain {
    parameters: Arc<MusicEffectChainParameters>,
    sample_rate: f64,
    current_reverb_preset: AVAudioUnitReverbPreset,
    spin_phase: f64,
    is_spatialised: bool,
}

impl MusicEffectAudioChain {
    const ORBIT_RADIUS: f32 = 7.0;

    pub fn new(sample_rate: f64) -> Self {
        Self {
            parameters: Arc::new(MusicEffectChainParameters::new()),
            sample_rate,
            current_reverb_preset: AVAudioUnitReverbPreset::LargeHall,
            spin_phase: 0.0,
            is_spatialised: false,
        }
    }

    pub fn attach(
        &mut self,
        engine: &mut MusicEffectAudioEngine,
        source: &MusicEffectPlayerNode,
        format: StreamFormat,
        settings: &MusicAudioEffectSettings,
    ) {
        self.is_spatialised = settings.is_enabled(MusicAudioEffectKind::Spin);
        let renderer = MusicEffectChainRenderer::new(
            Arc::clone(&self.parameters),
            format,
            self.is_spatialised,
            self.sample_rate,
        );
        engine.attach(MusicEffectGraph {
            player: source.render_side(format),
            renderer,
            format,
        });
    }

    pub fn requires_rebuild(&self, settings: &MusicAudioEffectSettings) -> bool {
        settings.is_enabled(MusicAudioEffectKind::Spin) != self.is_spatialised
    }

    pub fn spatialised(&self) -> bool {
        self.is_spatialised
    }

    pub fn apply(&mut self, settings: &MusicAudioEffectSettings) {
        let parameters = &self.parameters;
        parameters.varispeed_rate.store(settings.playback_rate() as f32);

        parameters.pitch_cents.store(MusicAudioEffectDSP::pitch_cents(settings));
        parameters
            .pitch_bypass
            .store(!settings.is_enabled(MusicAudioEffectKind::Pitch), Ordering::Relaxed);

        parameters.low_pass_hz.store(MusicAudioEffectDSP::low_pass_hz(settings));
        parameters
            .low_pass_bypass
            .store(!MusicAudioEffectDSP::is_low_pass_active(settings), Ordering::Relaxed);
        parameters
            .low_shelf_gain_db
            .store(MusicAudioEffectDSP::low_shelf_gain_db(settings));
        parameters
            .low_shelf_bypass
            .store(!MusicAudioEffectDSP::is_low_shelf_active(settings), Ordering::Relaxed);
        parameters.high_pass_hz.store(MusicAudioEffectDSP::high_pass_hz(settings));
        parameters
            .high_pass_bypass
            .store(!MusicAudioEffectDSP::is_high_pass_active(settings), Ordering::Relaxed);
        parameters
            .global_gain_db
            .store(Self::decibels(MusicAudioEffectDSP::direct_gain(settings)));

        parameters
            .delay_wet_dry_mix
            .store(MusicAudioEffectDSP::delay_wet_dry_mix(settings));
        parameters
            .delay_feedback
            .store(MusicAudioEffectDSP::delay_feedback(settings));
        parameters
            .delay_bypass
            .store(!settings.is_enabled(MusicAudioEffectKind::Echo), Ordering::Relaxed);

        self.apply_dynamics(settings);

        parameters.vinyl_gain.store(MusicAudioEffectDSP::ambience_gain(
            settings,
            MusicAudioEffectKind::Vinyl,
        ));
    }

    fn apply_dynamics(&self, settings: &MusicAudioEffectSettings) {
        let is_on = settings.is_enabled(MusicAudioEffectKind::NightMode);
        self.parameters.dynamics_bypass.store(!is_on, Ordering::Relaxed);
        if !is_on {
            return;
        }
        self.parameters
            .compression_threshold_db
            .store(MusicAudioEffectDSP::compression_threshold_db(settings));
        self.parameters
            .compression_makeup_db
            .store(MusicAudioEffectDSP::compression_makeup_db(settings));
    }

    /// swapping a reverb preset on a live graph is audible so the wet signal is pulled down first
    /// the caller ramps it back after
    pub fn update_reverb_preset(&mut self, settings: &MusicAudioEffectSettings) -> bool {
        let preset = MusicAudioEffectDSP::reverb_preset(settings);
        if preset == self.current_reverb_preset {
            return false;
        }
        self.current_reverb_preset = preset;
        self.parameters.reverb_wet_dry_mix.store(0.0);
        self.parameters
            .reverb_preset
            .store(reverb_preset_raw(preset), Ordering::Relaxed);
        true
    }

    pub fn set_reverb_mix(&self, mix: f32) {
        self.parameters.reverb_wet_dry_mix.store(mix.clamp(0.0, 100.0));
    }

    pub fn reverb_wet_dry_mix(&self) -> f32 {
        self.parameters.reverb_wet_dry_mix.load()
    }

    pub fn reverb_tail_time(&self) -> f64 {
        MusicEffectReverb::tail_time(self.current_reverb_preset)
    }

    /// 8D, the source orbits the listener a few metres out
    /// advancing by elapsed time keeps rotation identical in realtime and in the exporter
    pub fn advance_spin(&mut self, seconds: f64, settings: &MusicAudioEffectSettings) {
        if !self.is_spatialised {
            return;
        }
        let hz = MusicAudioEffectDSP::spin_hz(settings);
        if hz <= 0.0 {
            return;
        }
        self.spin_phase += hz * seconds;
        let angle = (self.spin_phase * 2.0 * std::f64::consts::PI) as f32;
        let radius = Self::ORBIT_RADIUS;
        self.parameters.submix_position_x.store(radius * angle.sin());
        self.parameters.submix_position_z.store(-radius * angle.cos());
    }

    /// ambience is generated and not scheduled so pausing the player would leave it crackling over silence
    pub fn set_ambience_muted(&self, muted: bool) {
        self.parameters.ambience_muted.store(muted, Ordering::Relaxed);
    }

    fn decibels(gain: f32) -> f32 {
        if gain <= 0.0001 {
            return -96.0;
        }
        20.0 * gain.log10()
    }
}

/// biquad filter, transposed direct form II one state per channel
#[derive(Clone, Copy, Debug, Default)]
struct MusicEffectBiquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
}

#[derive(Clone, Copy, Debug, Default)]
struct MusicEffectBiquadState {
    z1: f32,
    z2: f32,
}

impl MusicEffectBiquad {
    const BUTTERWORTH_Q: f32 = std::f32::consts::FRAC_1_SQRT_2;

    fn normalized(b0: f32, b1: f32, b2: f32, a0: f32, a1: f32, a2: f32) -> Self {
        Self {
            b0: b0 / a0,
            b1: b1 / a0,
            b2: b2 / a0,
            a1: a1 / a0,
            a2: a2 / a0,
        }
    }

    fn omega(sample_rate: f32, frequency: f32) -> (f32, f32) {
        let frequency = frequency.clamp(10.0, sample_rate * 0.45);
        let omega = 2.0 * PI * frequency / sample_rate;
        (omega.sin(), omega.cos())
    }

    fn low_pass(sample_rate: f32, frequency: f32) -> Self {
        let (sin, cos) = Self::omega(sample_rate, frequency);
        let alpha = sin / (2.0 * Self::BUTTERWORTH_Q);
        Self::normalized(
            (1.0 - cos) / 2.0,
            1.0 - cos,
            (1.0 - cos) / 2.0,
            1.0 + alpha,
            -2.0 * cos,
            1.0 - alpha,
        )
    }

    fn high_pass(sample_rate: f32, frequency: f32) -> Self {
        let (sin, cos) = Self::omega(sample_rate, frequency);
        let alpha = sin / (2.0 * Self::BUTTERWORTH_Q);
        Self::normalized(
            (1.0 + cos) / 2.0,
            -(1.0 + cos),
            (1.0 + cos) / 2.0,
            1.0 + alpha,
            -2.0 * cos,
            1.0 - alpha,
        )
    }

    fn low_shelf(sample_rate: f32, frequency: f32, gain_db: f32) -> Self {
        let (sin, cos) = Self::omega(sample_rate, frequency);
        let a = 10f32.powf(gain_db / 40.0);
        let alpha = sin / 2.0 * std::f32::consts::SQRT_2;
        let root = 2.0 * a.sqrt() * alpha;
        Self::normalized(
            a * ((a + 1.0) - (a - 1.0) * cos + root),
            2.0 * a * ((a - 1.0) - (a + 1.0) * cos),
            a * ((a + 1.0) - (a - 1.0) * cos - root),
            (a + 1.0) + (a - 1.0) * cos + root,
            -2.0 * ((a - 1.0) + (a + 1.0) * cos),
            (a + 1.0) + (a - 1.0) * cos - root,
        )
    }

    fn process(&self, state: &mut MusicEffectBiquadState, input: f32) -> f32 {
        let output = self.b0 * input + state.z1;
        state.z1 = self.b1 * input - self.a1 * output + state.z2;
        state.z2 = self.b2 * input - self.a2 * output;
        output
    }
}

/// pitch only time pitch
/// two read heads sweep a short delay line at the pitch ratio and crossfade
/// voice moves without the timing moving
struct MusicEffectPitchShifter {
    buffer: Vec<f32>,
    write: usize,
    phase: f32,
    window: f32,
}

impl MusicEffectPitchShifter {
    const WINDOW_SECONDS: f32 = 0.046;

    fn new(sample_rate: f32) -> Self {
        let window = (Self::WINDOW_SECONDS * sample_rate).max(64.0);
        Self {
            buffer: vec![0.0; window as usize + 4],
            write: 0,
            phase: 0.0,
            window,
        }
    }

    fn tap(&self, phase: f32) -> f32 {
        let delay = phase * self.window + 1.0;
        let length = self.buffer.len() as f32;
        let mut position = self.write as f32 - delay;
        if position < 0.0 {
            position += length;
        }
        let index = position.floor() as usize % self.buffer.len();
        let next = (index + 1) % self.buffer.len();
        let fraction = position - position.floor();
        self.buffer[index] + (self.buffer[next] - self.buffer[index]) * fraction
    }

    fn process(&mut self, input: f32, ratio: f32) -> f32 {
        self.buffer[self.write] = input;
        self.phase += (1.0 - ratio) / self.window;
        self.phase -= self.phase.floor();
        let other = (self.phase + 0.5).fract();
        let output = self.tap(self.phase) * (PI * self.phase).sin() + self.tap(other) * (PI * other).sin();
        self.write = (self.write + 1) % self.buffer.len();
        output
    }
}

/// delay unit, one delay line per channel with the low pass in the feedback path
struct MusicEffectDelayLine {
    buffer: Vec<f32>,
    index: usize,
    low_pass: f32,
}

impl MusicEffectDelayLine {
    const LOW_PASS_CUTOFF_HZ: f32 = 6_000.0;

    fn new(sample_rate: f32) -> Self {
        let length = (MusicAudioEffectDSP::DELAY_TIME_SECONDS as f32 * sample_rate).ceil() as usize;
        Self {
            buffer: vec![0.0; length.max(1)],
            index: 0,
            low_pass: 0.0,
        }
    }

    fn process(&mut self, input: f32, wet: f32, feedback: f32, low_pass_coefficient: f32) -> f32 {
        let delayed = self.buffer[self.index];
        self.low_pass += low_pass_coefficient * (delayed - self.low_pass);
        self.buffer[self.index] = input + self.low_pass * feedback;
        self.index = (self.index + 1) % self.buffer.len();
        input * (1.0 - wet) + self.low_pass * wet
    }
}

struct MusicEffectReverbComb {
    buffer: Vec<f32>,
    index: usize,
    filter_store: f32,
}

impl MusicEffectReverbComb {
    fn new(length: usize) -> Self {
        Self {
            buffer: vec![0.0; length.max(1)],
            index: 0,
            filter_store: 0.0,
        }
    }

    fn process(&mut self, input: f32, feedback: f32, damp: f32) -> f32 {
        let output = self.buffer[self.index];
        self.filter_store = output * (1.0 - damp) + self.filter_store * damp;
        self.buffer[self.index] = input + self.filter_store * feedback;
        self.index = (self.index + 1) % self.buffer.len();
        output
    }
}

struct MusicEffectReverbAllpass {
    buffer: Vec<f32>,
    index: usize,
}

impl MusicEffectReverbAllpass {
    fn new(length: usize) -> Self {
        Self {
            buffer: vec![0.0; length.max(1)],
            index: 0,
        }
    }

    fn process(&mut self, input: f32) -> f32 {
        let buffered = self.buffer[self.index];
        self.buffer[self.index] = input + buffered * 0.5;
        self.index = (self.index + 1) % self.buffer.len();
        buffered - input
    }
}

/// reverb with factory presets, a stereo schroeder moorer network
/// room size and damping stand for the preset
struct MusicEffectReverb {
    combs_left: Vec<MusicEffectReverbComb>,
    combs_right: Vec<MusicEffectReverbComb>,
    allpasses_left: Vec<MusicEffectReverbAllpass>,
    allpasses_right: Vec<MusicEffectReverbAllpass>,
    feedback: f32,
    damp: f32,
}

impl MusicEffectReverb {
    const COMB_TUNING: [usize; 8] = [1116, 1188, 1277, 1356, 1422, 1491, 1557, 1617];
    const ALLPASS_TUNING: [usize; 4] = [556, 441, 341, 225];
    const STEREO_SPREAD: usize = 23;
    const TUNING_RATE: f64 = 44_100.0;
    const INPUT_GAIN: f32 = 0.015;
    const WET_SCALE: f32 = 3.0;

    fn new(sample_rate: f64, preset: AVAudioUnitReverbPreset) -> Self {
        let scale = sample_rate / Self::TUNING_RATE;
        let scaled = |length: usize| ((length as f64) * scale).round() as usize;
        let (feedback, damp) = Self::voicing(preset);
        Self {
            combs_left: Self::COMB_TUNING
                .iter()
                .map(|length| MusicEffectReverbComb::new(scaled(*length)))
                .collect(),
            combs_right: Self::COMB_TUNING
                .iter()
                .map(|length| MusicEffectReverbComb::new(scaled(*length + Self::STEREO_SPREAD)))
                .collect(),
            allpasses_left: Self::ALLPASS_TUNING
                .iter()
                .map(|length| MusicEffectReverbAllpass::new(scaled(*length)))
                .collect(),
            allpasses_right: Self::ALLPASS_TUNING
                .iter()
                .map(|length| MusicEffectReverbAllpass::new(scaled(*length + Self::STEREO_SPREAD)))
                .collect(),
            feedback,
            damp,
        }
    }

    /// room size and damping per preset, as comb feedback and one pole damping
    fn voicing(preset: AVAudioUnitReverbPreset) -> (f32, f32) {
        let (room_size, damping) = match preset {
            AVAudioUnitReverbPreset::MediumChamber => (0.70, 0.50),
            AVAudioUnitReverbPreset::MediumHall => (0.78, 0.40),
            AVAudioUnitReverbPreset::LargeHall => (0.85, 0.30),
            AVAudioUnitReverbPreset::Cathedral => (0.93, 0.20),
        };
        (room_size * 0.28 + 0.7, damping * 0.4)
    }

    fn tail_time(preset: AVAudioUnitReverbPreset) -> f64 {
        let (feedback, _) = Self::voicing(preset);
        let longest = Self::COMB_TUNING[Self::COMB_TUNING.len() - 1] + Self::STEREO_SPREAD;
        let loop_seconds = longest as f64 / Self::TUNING_RATE;
        -3.0 * loop_seconds / f64::from(feedback).log10()
    }

    fn load_factory_preset(&mut self, preset: AVAudioUnitReverbPreset) {
        let (feedback, damp) = Self::voicing(preset);
        self.feedback = feedback;
        self.damp = damp;
    }

    fn process(&mut self, input: f32) -> (f32, f32) {
        let input = input * Self::INPUT_GAIN;
        let (feedback, damp) = (self.feedback, self.damp);
        let mut left: f32 = self
            .combs_left
            .iter_mut()
            .map(|comb| comb.process(input, feedback, damp))
            .sum();
        let mut right: f32 = self
            .combs_right
            .iter_mut()
            .map(|comb| comb.process(input, feedback, damp))
            .sum();
        for allpass in &mut self.allpasses_left {
            left = allpass.process(left);
        }
        for allpass in &mut self.allpasses_right {
            right = allpass.process(right);
        }
        (left * Self::WET_SCALE, right * Self::WET_SCALE)
    }
}

/// point source environment node
/// inverse distance attenuation, interaural delay and head shadow by azimuth
/// and a medium hall room around the listener
struct MusicEffectEnvironment {
    delay_left: Vec<f32>,
    delay_right: Vec<f32>,
    index: usize,
    shadow_left: f32,
    shadow_right: f32,
    behind_left: f32,
    behind_right: f32,
    room: MusicEffectReverb,
    sample_rate: f32,
}

impl MusicEffectEnvironment {
    const REFERENCE_DISTANCE: f32 = 3.0;
    const MAXIMUM_DISTANCE: f32 = 60.0;
    const ROLLOFF_FACTOR: f32 = 1.1;
    const ROOM_LEVEL_DB: f32 = -13.0;
    const HEAD_RADIUS_METRES: f32 = 0.0875;
    const SPEED_OF_SOUND: f32 = 343.0;
    const DELAY_LINE: usize = 128;

    fn new(sample_rate: f32) -> Self {
        Self {
            delay_left: vec![0.0; Self::DELAY_LINE],
            delay_right: vec![0.0; Self::DELAY_LINE],
            index: 0,
            shadow_left: 0.0,
            shadow_right: 0.0,
            behind_left: 0.0,
            behind_right: 0.0,
            room: MusicEffectReverb::new(f64::from(sample_rate), AVAudioUnitReverbPreset::MediumHall),
            sample_rate,
        }
    }

    fn one_pole(sample_rate: f32, cutoff: f32) -> f32 {
        1.0 - (-2.0 * PI * cutoff / sample_rate).exp()
    }

    fn read(line: &[f32], index: usize, delay: f32) -> f32 {
        let length = line.len() as f32;
        let mut position = index as f32 - delay;
        if position < 0.0 {
            position += length;
        }
        let base = position.floor() as usize % line.len();
        let next = (base + 1) % line.len();
        let fraction = position - position.floor();
        line[base] + (line[next] - line[base]) * fraction
    }

    /// renders a mono block at x and z into interleaved stereo
    fn process(&mut self, mono: &[f32], x: f32, z: f32, stereo: &mut Vec<f32>) {
        let distance = (x * x + z * z).sqrt().clamp(Self::REFERENCE_DISTANCE, Self::MAXIMUM_DISTANCE);
        let attenuation = Self::REFERENCE_DISTANCE
            / (Self::REFERENCE_DISTANCE
                + Self::ROLLOFF_FACTOR * (distance - Self::REFERENCE_DISTANCE));
        let azimuth = x.atan2(-z);
        let lateral = azimuth.sin();
        let lateral_angle = lateral.abs().asin();
        let interaural_seconds = Self::HEAD_RADIUS_METRES / Self::SPEED_OF_SOUND
            * (lateral_angle + lateral_angle.sin());
        let interaural_frames =
            (interaural_seconds * self.sample_rate).min(Self::DELAY_LINE as f32 - 2.0);
        let far_gain = 1.0 - 0.35 * lateral.abs();
        let shadow = Self::one_pole(self.sample_rate, 20_000.0 - 17_500.0 * lateral.abs());
        let behind = Self::one_pole(
            self.sample_rate,
            20_000.0 - 14_000.0 * (-azimuth.cos()).max(0.0),
        );
        let room_level = 10f32.powf(Self::ROOM_LEVEL_DB / 20.0);
        let (left_delay, right_delay) = if lateral >= 0.0 {
            (interaural_frames, 0.0)
        } else {
            (0.0, interaural_frames)
        };

        for sample in mono {
            let direct = sample * attenuation;
            self.delay_left[self.index] = direct;
            self.delay_right[self.index] = direct;
            let mut left = Self::read(&self.delay_left, self.index, left_delay);
            let mut right = Self::read(&self.delay_right, self.index, right_delay);
            if lateral >= 0.0 {
                self.shadow_left += shadow * (left - self.shadow_left);
                left = self.shadow_left * far_gain;
            } else {
                self.shadow_right += shadow * (right - self.shadow_right);
                right = self.shadow_right * far_gain;
            }
            self.behind_left += behind * (left - self.behind_left);
            self.behind_right += behind * (right - self.behind_right);
            let (room_left, room_right) = self.room.process(direct);
            stereo.push(self.behind_left + room_left * room_level);
            stereo.push(self.behind_right + room_right * room_level);
            self.index = (self.index + 1) % Self::DELAY_LINE;
        }
    }
}

/// render side of the chain, owns every units state and reads
/// the params the main thread set once per block
struct MusicEffectChainRenderer {
    parameters: Arc<MusicEffectChainParameters>,
    channels: usize,
    sample_rate: f32,
    spatialised: bool,

    varispeed_input: Vec<f32>,
    varispeed_position: f64,
    varispeed_rate: f32,
    pitch: Vec<MusicEffectPitchShifter>,
    eq_low_pass: MusicEffectBiquad,
    eq_low_shelf: MusicEffectBiquad,
    eq_high_pass: MusicEffectBiquad,
    eq_key: (f32, f32, f32),
    eq_state: Vec<[MusicEffectBiquadState; 3]>,
    delay: Vec<MusicEffectDelayLine>,
    reverb: MusicEffectReverb,
    reverb_preset: AVAudioUnitReverbPreset,
    dynamics_envelope: f32,
    environment: MusicEffectEnvironment,
    ambience: MusicAmbienceGenerator,

    pulled: Vec<f32>,
    block: Vec<f32>,
    mono: Vec<f32>,
    stereo: Vec<f32>,
}

impl MusicEffectChainRenderer {
    const DYNAMICS_HEADROOM_DB: f32 = 5.0;
    const DYNAMICS_ATTACK_SECONDS: f32 = 0.001;
    const DYNAMICS_RELEASE_SECONDS: f32 = 0.05;
    const SPATIAL_CHANNELS: u16 = 2;

    fn new(
        parameters: Arc<MusicEffectChainParameters>,
        format: StreamFormat,
        spatialised: bool,
        sample_rate: f64,
    ) -> Self {
        let channels = format.channels as usize;
        let rate = sample_rate as f32;
        let preset = reverb_preset_from_raw(parameters.reverb_preset.load(Ordering::Relaxed));
        Self {
            parameters,
            channels,
            sample_rate: rate,
            spatialised,
            varispeed_input: Vec::new(),
            varispeed_position: 0.0,
            varispeed_rate: 1.0,
            pitch: (0..channels).map(|_| MusicEffectPitchShifter::new(rate)).collect(),
            eq_low_pass: MusicEffectBiquad::default(),
            eq_low_shelf: MusicEffectBiquad::default(),
            eq_high_pass: MusicEffectBiquad::default(),
            eq_key: (f32::NAN, f32::NAN, f32::NAN),
            eq_state: vec![[MusicEffectBiquadState::default(); 3]; channels],
            delay: (0..channels).map(|_| MusicEffectDelayLine::new(rate)).collect(),
            reverb: MusicEffectReverb::new(sample_rate, preset),
            reverb_preset: preset,
            dynamics_envelope: 0.0,
            environment: MusicEffectEnvironment::new(rate),
            ambience: MusicAmbienceGenerator::new(sample_rate),
            pulled: Vec::new(),
            block: Vec::new(),
            mono: Vec::new(),
            stereo: Vec::new(),
        }
    }

    fn render(&mut self, player: &mut MusicEffectPlayerNodeRender, frames: usize, out: &mut Vec<f32>) {
        let channels = self.channels;
        self.varispeed(player, frames);
        self.time_pitch();
        self.equalize();
        self.echo();
        self.reverberate();
        self.compress();

        let start = out.len();
        if self.spatialised {
            self.mono.clear();
            for frame in self.block.chunks_exact(channels) {
                self.mono.push(frame.iter().sum::<f32>() / channels as f32);
            }
            self.stereo.clear();
            let x = self.parameters.submix_position_x.load();
            let z = self.parameters.submix_position_z.load();
            self.environment.process(&self.mono, x, z, &mut self.stereo);
            map_channels(&self.stereo, Self::SPATIAL_CHANNELS, channels as u16, out);
        } else {
            out.extend_from_slice(&self.block);
        }

        self.ambience.vinyl_gain = self.parameters.vinyl_gain.load();
        self.ambience.is_muted = self.parameters.ambience_muted.load(Ordering::Relaxed);
        if !self.ambience.is_silent() {
            for frame in out[start..].chunks_exact_mut(channels) {
                let value = self.ambience.next_sample();
                for sample in frame {
                    *sample += value;
                }
            }
        }
    }

    fn varispeed(&mut self, player: &mut MusicEffectPlayerNodeRender, frames: usize) {
        let channels = self.channels;
        let target = self.parameters.varispeed_rate.load().clamp(0.25, 4.0);
        let start_rate = self.varispeed_rate;
        let step = (target - start_rate) / frames.max(1) as f32;
        let needed = (self.varispeed_position + f64::from(start_rate.max(target)) * frames as f64)
            .ceil() as usize
            + 2;
        let buffered = self.varispeed_input.len() / channels;
        if needed > buffered {
            self.pulled.clear();
            player.pull(needed - buffered, &mut self.pulled);
            self.varispeed_input.extend_from_slice(&self.pulled);
        }

        self.block.clear();
        let mut position = self.varispeed_position;
        let mut rate = start_rate;
        let available = self.varispeed_input.len() / channels;
        for _ in 0..frames {
            let index = position.floor() as usize;
            let fraction = (position - position.floor()) as f32;
            for channel in 0..channels {
                let current = if index < available {
                    self.varispeed_input[index * channels + channel]
                } else {
                    0.0
                };
                let next = if index + 1 < available {
                    self.varispeed_input[(index + 1) * channels + channel]
                } else {
                    current
                };
                self.block.push(current + (next - current) * fraction);
            }
            rate += step;
            position += f64::from(rate);
        }
        self.varispeed_rate = target;

        let consumed = (position.floor() as usize).min(available);
        self.varispeed_input.drain(..consumed * channels);
        self.varispeed_position = position - consumed as f64;
    }

    fn time_pitch(&mut self) {
        if self.parameters.pitch_bypass.load(Ordering::Relaxed) {
            return;
        }
        let ratio = 2f32.powf(self.parameters.pitch_cents.load() / 1_200.0);
        let channels = self.channels;
        for frame in self.block.chunks_exact_mut(channels) {
            for (sample, shifter) in frame.iter_mut().zip(self.pitch.iter_mut()) {
                *sample = shifter.process(*sample, ratio);
            }
        }
    }

    fn equalize(&mut self) {
        let parameters = &self.parameters;
        let key = (
            parameters.low_pass_hz.load(),
            parameters.low_shelf_gain_db.load(),
            parameters.high_pass_hz.load(),
        );
        if key != self.eq_key {
            self.eq_key = key;
            self.eq_low_pass = MusicEffectBiquad::low_pass(self.sample_rate, key.0);
            self.eq_low_shelf = MusicEffectBiquad::low_shelf(self.sample_rate, 160.0, key.1);
            self.eq_high_pass = MusicEffectBiquad::high_pass(self.sample_rate, key.2);
        }
        let low_pass_on = !parameters.low_pass_bypass.load(Ordering::Relaxed);
        let low_shelf_on = !parameters.low_shelf_bypass.load(Ordering::Relaxed);
        let high_pass_on = !parameters.high_pass_bypass.load(Ordering::Relaxed);
        let global_gain = 10f32.powf(parameters.global_gain_db.load() / 20.0);
        let channels = self.channels;
        for frame in self.block.chunks_exact_mut(channels) {
            for (sample, state) in frame.iter_mut().zip(self.eq_state.iter_mut()) {
                let mut value = *sample;
                if low_pass_on {
                    value = self.eq_low_pass.process(&mut state[0], value);
                }
                if low_shelf_on {
                    value = self.eq_low_shelf.process(&mut state[1], value);
                }
                if high_pass_on {
                    value = self.eq_high_pass.process(&mut state[2], value);
                }
                *sample = value * global_gain;
            }
        }
    }

    fn echo(&mut self) {
        if self.parameters.delay_bypass.load(Ordering::Relaxed) {
            return;
        }
        let wet = self.parameters.delay_wet_dry_mix.load() / 100.0;
        let feedback = (self.parameters.delay_feedback.load() / 100.0).clamp(-0.99, 0.99);
        let coefficient = MusicEffectEnvironment::one_pole(
            self.sample_rate,
            MusicEffectDelayLine::LOW_PASS_CUTOFF_HZ,
        );
        let channels = self.channels;
        for frame in self.block.chunks_exact_mut(channels) {
            for (sample, line) in frame.iter_mut().zip(self.delay.iter_mut()) {
                *sample = line.process(*sample, wet, feedback, coefficient);
            }
        }
    }

    fn reverberate(&mut self) {
        let preset = reverb_preset_from_raw(self.parameters.reverb_preset.load(Ordering::Relaxed));
        if preset != self.reverb_preset {
            self.reverb_preset = preset;
            self.reverb.load_factory_preset(preset);
        }
        let wet = self.parameters.reverb_wet_dry_mix.load() / 100.0;
        let channels = self.channels;
        for frame in self.block.chunks_exact_mut(channels) {
            let input = frame.iter().sum::<f32>() / channels as f32;
            let (left, right) = self.reverb.process(input);
            for (channel, sample) in frame.iter_mut().enumerate() {
                let tail = match channel {
                    0 => left,
                    1 => right,
                    _ => (left + right) * 0.5,
                };
                let tail = if channels == 1 { (left + right) * 0.5 } else { tail };
                *sample = *sample * (1.0 - wet) + tail * wet;
            }
        }
    }

    fn compress(&mut self) {
        if self.parameters.dynamics_bypass.load(Ordering::Relaxed) {
            return;
        }
        let threshold = self.parameters.compression_threshold_db.load();
        let makeup = self.parameters.compression_makeup_db.load();
        let attack = (-1.0 / (Self::DYNAMICS_ATTACK_SECONDS * self.sample_rate)).exp();
        let release = (-1.0 / (Self::DYNAMICS_RELEASE_SECONDS * self.sample_rate)).exp();
        let channels = self.channels;
        for frame in self.block.chunks_exact_mut(channels) {
            let peak = frame.iter().fold(0f32, |peak, sample| peak.max(sample.abs()));
            let coefficient = if peak > self.dynamics_envelope { attack } else { release };
            self.dynamics_envelope = coefficient * self.dynamics_envelope + (1.0 - coefficient) * peak;
            let level_db = 20.0 * (self.dynamics_envelope + 1e-9).log10();
            let over = level_db - threshold;
            let reduction = if over > 0.0 {
                Self::DYNAMICS_HEADROOM_DB * (1.0 - (-over / Self::DYNAMICS_HEADROOM_DB).exp()) - over
            } else {
                0.0
            };
            let gain = 10f32.powf((reduction + makeup) / 20.0);
            for sample in frame {
                *sample *= gain;
            }
        }
    }
}

/// audio file opened for reading, whole track in memory and decoded on demand
#[derive(Clone)]
pub struct MusicEffectAudioFile {
    bytes: Arc<Vec<u8>>,
    processing_format: StreamFormat,
    length: i64,
    trim: Trim,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MusicEffectAudioFileError {
    Io(String),
    Decode(DecodeError),
}

impl MusicEffectAudioFile {
    /// opens a file for reading, blocking so call it on a worker
    pub fn for_reading(path: &Path) -> Result<Self, MusicEffectAudioFileError> {
        let bytes = std::fs::read(path).map_err(|error| MusicEffectAudioFileError::Io(error.to_string()))?;
        let mut decoder =
            SymphoniaDecoder::open(bytes.clone()).map_err(MusicEffectAudioFileError::Decode)?;
        let processing_format = decoder.format();
        let trim = decoder.trim();
        let length = match (trim.valid_frames, decoder.duration_millis()) {
            (Some(valid), _) => valid as i64,
            (None, Some(millis)) => processing_format.millis_to_frames(millis) as i64,
            (None, None) => Self::count_frames(&mut decoder)?,
        };
        Ok(Self {
            bytes: Arc::new(bytes),
            processing_format,
            length,
            trim,
        })
    }

    fn count_frames(decoder: &mut SymphoniaDecoder) -> Result<i64, MusicEffectAudioFileError> {
        let mut samples = Vec::new();
        let mut frames: i64 = 0;
        loop {
            samples.clear();
            match decoder.decode(&mut samples).map_err(MusicEffectAudioFileError::Decode)? {
                Decoded::Frames(count) => frames += count as i64,
                Decoded::EndOfStream => break,
            }
        }
        Ok(frames)
    }

    pub fn processing_format(&self) -> StreamFormat {
        self.processing_format
    }

    /// frames in the file at its own rate
    pub fn length(&self) -> i64 {
        self.length
    }

    fn open_decoder(&self) -> Result<SymphoniaDecoder, DecodeError> {
        SymphoniaDecoder::open(self.bytes.as_ref().clone())
    }
}

enum MusicEffectPlayerCommand {
    Schedule {
        file: MusicEffectAudioFile,
        starting_frame: i64,
        frame_count: i64,
        completion: Box<dyn FnOnce() + Send>,
    },
    Stop,
}

#[derive(Default)]
struct MusicEffectPlayerShared {
    volume: AtomicF32,
    is_playing: AtomicBool,
    commands: Mutex<VecDeque<MusicEffectPlayerCommand>>,
    sample_time: AtomicI64,
    has_render_time: AtomicBool,
}

/// player node, the handle the main thread schedules plays and pauses
/// the render side pulls the scheduled segment from it
#[derive(Clone)]
pub struct MusicEffectPlayerNode {
    shared: Arc<MusicEffectPlayerShared>,
}

impl Default for MusicEffectPlayerNode {
    fn default() -> Self {
        Self::new()
    }
}

impl MusicEffectPlayerNode {
    pub fn new() -> Self {
        let shared = MusicEffectPlayerShared::default();
        shared.volume.store(1.0);
        Self {
            shared: Arc::new(shared),
        }
    }

    pub fn volume(&self) -> f32 {
        self.shared.volume.load()
    }

    pub fn set_volume(&self, volume: f32) {
        self.shared.volume.store(volume);
    }

    pub fn is_playing(&self) -> bool {
        self.shared.is_playing.load(Ordering::Acquire)
    }

    pub fn play(&self) {
        self.shared.is_playing.store(true, Ordering::Release);
    }

    pub fn pause(&self) {
        self.shared.is_playing.store(false, Ordering::Release);
    }

    /// drops the schedule, the render side gives silence until the next one
    pub fn stop(&self) {
        self.shared.is_playing.store(false, Ordering::Release);
        self.shared.has_render_time.store(false, Ordering::Release);
        self.shared.sample_time.store(0, Ordering::Release);
        self.shared.commands.lock().push_back(MusicEffectPlayerCommand::Stop);
    }

    /// schedules a segment
    /// completion runs on the render side once the last frame of the segment has been played
    pub fn schedule_segment(
        &self,
        file: &MusicEffectAudioFile,
        starting_frame: i64,
        frame_count: i64,
        completion: impl FnOnce() + Send + 'static,
    ) {
        self.shared.sample_time.store(0, Ordering::Release);
        self.shared.commands.lock().push_back(MusicEffectPlayerCommand::Schedule {
            file: file.clone(),
            starting_frame,
            frame_count,
            completion: Box::new(completion),
        });
    }

    pub fn schedule_file(&self, file: &MusicEffectAudioFile) {
        self.schedule_segment(file, 0, file.length(), || {});
    }

    /// frames of the segment heard so far, none before the render side produced any
    pub fn player_sample_time(&self) -> Option<i64> {
        if !self.shared.has_render_time.load(Ordering::Acquire) {
            return None;
        }
        Some(self.shared.sample_time.load(Ordering::Acquire))
    }

    fn render_side(&self, format: StreamFormat) -> MusicEffectPlayerNodeRender {
        MusicEffectPlayerNodeRender {
            shared: Arc::clone(&self.shared),
            format,
            decoder: None,
            decoder_file: None,
            trimmer: Trimmer::new(Trim::NONE),
            decoded: VecDeque::new(),
            remaining: 0,
            skip: 0,
            pulled_frames: 0,
            completion: None,
            segment_ended: false,
        }
    }
}

struct MusicEffectPlayerNodeRender {
    shared: Arc<MusicEffectPlayerShared>,
    format: StreamFormat,
    decoder: Option<SymphoniaDecoder>,
    decoder_file: Option<Arc<Vec<u8>>>,
    trimmer: Trimmer,
    decoded: VecDeque<f32>,
    remaining: i64,
    skip: u64,
    pulled_frames: i64,
    completion: Option<Box<dyn FnOnce() + Send>>,
    segment_ended: bool,
}

impl MusicEffectPlayerNodeRender {
    fn take_commands(&mut self) {
        let commands: Vec<MusicEffectPlayerCommand> = self.shared.commands.lock().drain(..).collect();
        for command in commands {
            match command {
                MusicEffectPlayerCommand::Stop => {
                    self.remaining = 0;
                    self.decoded.clear();
                    self.completion = None;
                    self.segment_ended = false;
                    self.pulled_frames = 0;
                }
                MusicEffectPlayerCommand::Schedule {
                    file,
                    starting_frame,
                    frame_count,
                    completion,
                } => {
                    self.decoded.clear();
                    self.pulled_frames = 0;
                    self.segment_ended = false;
                    self.completion = Some(completion);
                    self.remaining = frame_count.max(0);
                    if let Err(error) = self.seek(&file, starting_frame.max(0) as u64) {
                        tracing::warn!(target: "riseonly", ?error, "effect player could not schedule the segment");
                        self.remaining = 0;
                    }
                }
            }
        }
    }

    fn seek(&mut self, file: &MusicEffectAudioFile, frame: u64) -> Result<(), DecodeError> {
        let same_file = self
            .decoder_file
            .as_ref()
            .is_some_and(|bytes| Arc::ptr_eq(bytes, &file.bytes));
        if !same_file || self.decoder.is_none() {
            self.decoder = Some(file.open_decoder()?);
            self.decoder_file = Some(Arc::clone(&file.bytes));
            self.trimmer = Trimmer::new(file.trim);
        }
        let format = self.format;
        let Some(decoder) = self.decoder.as_mut() else {
            return Ok(());
        };
        let millis = format.frames_to_millis(frame);
        let reached = decoder.seek(millis)?;
        let reached_frame = format.millis_to_frames(reached);
        self.trimmer.seek_to(reached_frame);
        self.skip = frame.saturating_sub(reached_frame);
        Ok(())
    }

    /// appends exactly frames frames, the segment while playing and silence otherwise
    fn pull(&mut self, frames: usize, out: &mut Vec<f32>) {
        self.take_commands();
        let channels = self.format.channels as usize;
        let volume = self.shared.volume.load();
        if !self.shared.is_playing.load(Ordering::Acquire) {
            out.resize(out.len() + frames * channels, 0.0);
            return;
        }
        let mut produced = 0usize;
        while produced < frames && self.remaining > 0 {
            if self.decoded.is_empty() && !self.decode_more(channels) {
                self.remaining = 0;
                break;
            }
            let available = (self.decoded.len() / channels)
                .min(frames - produced)
                .min(self.remaining as usize);
            for sample in self.decoded.drain(..available * channels) {
                out.push(sample * volume);
            }
            produced += available;
            self.remaining -= available as i64;
            self.pulled_frames += available as i64;
        }
        if self.remaining <= 0 && self.completion.is_some() {
            self.segment_ended = true;
        }
        out.resize(out.len() + (frames - produced) * channels, 0.0);
    }

    fn decode_more(&mut self, channels: usize) -> bool {
        let Some(decoder) = self.decoder.as_mut() else {
            return false;
        };
        let mut block = Vec::new();
        loop {
            if self.trimmer.is_complete() {
                return false;
            }
            block.clear();
            match decoder.decode(&mut block) {
                Ok(Decoded::EndOfStream) => return false,
                Ok(Decoded::Frames(0)) => continue,
                Ok(Decoded::Frames(_)) => {}
                Err(error) => {
                    tracing::warn!(target: "riseonly", ?error, "effect player decode failed");
                    return false;
                }
            }
            let kept = self.trimmer.accept(block.len() / channels);
            if kept.is_empty() {
                continue;
            }
            let mut start = kept.start;
            if self.skip > 0 {
                let dropped = (self.skip as usize).min(kept.len());
                start += dropped;
                self.skip -= dropped as u64;
            }
            if start >= kept.end {
                continue;
            }
            self.decoded
                .extend(block[start * channels..kept.end * channels].iter().copied());
            return true;
        }
    }

    fn take_finished_completion(&mut self) -> Option<Box<dyn FnOnce() + Send>> {
        if !self.segment_ended {
            return None;
        }
        self.segment_ended = false;
        self.completion.take()
    }
}

/// the player and the chain put together
struct MusicEffectGraph {
    player: MusicEffectPlayerNodeRender,
    renderer: MusicEffectChainRenderer,
    format: StreamFormat,
}

impl MusicEffectGraph {
    fn render(&mut self, frames: usize, out: &mut Vec<f32>) {
        self.renderer.render(&mut self.player, frames, out);
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MusicEffectAudioEngineError {
    NoGraph,
    Output(OutputError),
    Thread(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AVAudioEngineManualRenderingStatus {
    Success,
    InsufficientDataFromInputNode,
    CannotDoInCurrentContext,
    Error,
}

enum MusicEffectEngineCommand {
    Stop,
}

struct MusicEffectEngineThread {
    commands: mpsc::Sender<MusicEffectEngineCommand>,
    handle: JoinHandle<MusicEffectGraph>,
}

/// engine for the effect graph
/// realtime means a render thread pulls the graph and feeds the cpal sink through the ring
/// manual means the caller pulls blocks itself faster than realtime, for the exporter
pub struct MusicEffectAudioEngine {
    graph: Option<MusicEffectGraph>,
    running: Option<MusicEffectEngineThread>,
    manual_rendering: Option<(StreamFormat, usize)>,
    manual_rendering_sample_time: i64,
    preferred_io_buffer_duration: f64,
    main_mixer_output_volume: Arc<AtomicF32>,
}

impl Default for MusicEffectAudioEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl MusicEffectAudioEngine {
    const DEFAULT_IO_BUFFER_DURATION: f64 = 0.046;
    const RENDER_BLOCK_FRAMES: usize = 512;
    const IDLE_WAIT: Duration = Duration::from_millis(2);

    pub fn new() -> Self {
        Self {
            graph: None,
            running: None,
            manual_rendering: None,
            manual_rendering_sample_time: 0,
            preferred_io_buffer_duration: Self::DEFAULT_IO_BUFFER_DURATION,
            main_mixer_output_volume: Arc::new(AtomicF32::new(1.0)),
        }
    }

    pub fn main_mixer_output_volume(&self) -> f32 {
        self.main_mixer_output_volume.load()
    }

    /// sets main mixer output volume, takes effect on the next rendered block
    pub fn set_main_mixer_output_volume(&self, volume: f32) {
        self.main_mixer_output_volume.store(volume.clamp(0.0, 1.0));
    }

    /// preferred io buffer duration as the playback session asks for it, the device ring holds two of these
    pub fn set_preferred_io_buffer_duration(&mut self, seconds: f64) {
        self.preferred_io_buffer_duration = seconds.clamp(0.005, 0.5);
    }

    fn attach(&mut self, graph: MusicEffectGraph) {
        self.graph = Some(graph);
    }

    /// prepare, the graph is already allocated when attached
    pub fn prepare(&mut self) {}

    pub fn is_running(&self) -> bool {
        self.running
            .as_ref()
            .is_some_and(|thread| !thread.handle.is_finished())
    }

    /// opens the default output device and starts rendering
    pub fn start(&mut self) -> Result<(), MusicEffectAudioEngineError> {
        if self.is_running() {
            return Ok(());
        }
        self.reclaim_graph();
        let graph = self.graph.take().ok_or(MusicEffectAudioEngineError::NoGraph)?;
        let format = graph.format;
        let ring_seconds = self.preferred_io_buffer_duration * 2.0;
        let (commands, inbox) = mpsc::channel::<MusicEffectEngineCommand>();
        let (opened, opened_inbox) = mpsc::channel::<Result<(), OutputError>>();
        let output_volume = self.main_mixer_output_volume.clone();
        let spawned = std::thread::Builder::new()
            .name("riseonly.music-effect-engine".to_owned())
            .spawn(move || Self::render_loop(graph, format, ring_seconds, output_volume, inbox, opened));
        let handle = spawned.map_err(|error| MusicEffectAudioEngineError::Thread(error.to_string()))?;
        match opened_inbox.recv() {
            Ok(Ok(())) => {
                self.running = Some(MusicEffectEngineThread { commands, handle });
                Ok(())
            }
            Ok(Err(error)) => {
                self.graph = handle.join().ok();
                Err(MusicEffectAudioEngineError::Output(error))
            }
            Err(_) => {
                self.graph = handle.join().ok();
                Err(MusicEffectAudioEngineError::Thread("render thread exited".to_owned()))
            }
        }
    }

    /// device closes but the graph and its schedule stay for the next start
    pub fn stop(&mut self) {
        if let Some(thread) = self.running.take() {
            let _ = thread.commands.send(MusicEffectEngineCommand::Stop);
            match thread.handle.join() {
                Ok(graph) => self.graph = Some(graph),
                Err(_) => tracing::error!(target: "riseonly", "effect render thread panicked"),
            }
        }
    }

    fn reclaim_graph(&mut self) {
        if self
            .running
            .as_ref()
            .is_some_and(|thread| thread.handle.is_finished())
            && let Some(thread) = self.running.take()
        {
            self.graph = thread.handle.join().ok();
        }
    }

    fn render_loop(
        mut graph: MusicEffectGraph,
        format: StreamFormat,
        ring_seconds: f64,
        output_volume: Arc<AtomicF32>,
        inbox: mpsc::Receiver<MusicEffectEngineCommand>,
        opened: mpsc::Sender<Result<(), OutputError>>,
    ) -> MusicEffectGraph {
        let guess_rate = f64::from(format.sample_rate.max(48_000));
        let guess_channels = f64::from(format.channels.max(2));
        let capacity = (guess_rate * ring_seconds * guess_channels * 2.0).ceil() as usize;
        let (mut producer, consumer) = ring(capacity);
        let mut sink = match CpalSink::open(format, consumer) {
            Ok(sink) => sink,
            Err(error) => {
                let _ = opened.send(Err(error));
                return graph;
            }
        };
        if let Err(error) = sink.play() {
            let _ = opened.send(Err(error));
            return graph;
        }
        let _ = opened.send(Ok(()));

        let device = sink.config();
        let target_samples = (f64::from(device.sample_rate) * ring_seconds * f64::from(device.channels))
            as usize;
        let mut resampler = device
            .resampling
            .then(|| Resampler::new(format, device.sample_rate));
        let mut rendered = Vec::new();
        let mut resampled = Vec::new();
        let mut mapped = Vec::new();
        let mut produced_device_frames: u64 = 0;
        let mut pending_completion: Option<(u64, Box<dyn FnOnce() + Send>)> = None;
        let device_per_source = f64::from(device.sample_rate) / f64::from(format.sample_rate);

        loop {
            match inbox.try_recv() {
                Ok(MusicEffectEngineCommand::Stop) | Err(mpsc::TryRecvError::Disconnected) => break,
                Err(mpsc::TryRecvError::Empty) => {}
            }

            if producer.filled() < target_samples.min(producer.capacity()) {
                rendered.clear();
                graph.render(Self::RENDER_BLOCK_FRAMES, &mut rendered);
                let volume = output_volume.load();
                if volume < 1.0 {
                    rendered.iter_mut().for_each(|sample| *sample *= volume);
                }
                if let Some(completion) = graph.player.take_finished_completion() {
                    let end = produced_device_frames
                        + (Self::RENDER_BLOCK_FRAMES as f64 * device_per_source) as u64;
                    pending_completion = Some((end, completion));
                }
                Self::push(
                    &rendered,
                    format,
                    device.channels,
                    resampler.as_mut(),
                    &mut resampled,
                    &mut mapped,
                    &mut producer,
                    &mut produced_device_frames,
                );
            } else {
                std::thread::sleep(Self::IDLE_WAIT);
            }

            Self::publish_sample_time(&graph, &producer, device.channels, device_per_source);

            let played = sink.frames_played();
            if pending_completion
                .as_ref()
                .is_some_and(|(end, _)| played >= *end)
                && let Some((_, completion)) = pending_completion.take()
            {
                completion();
            }
        }
        let _ = sink.pause();
        graph
    }

    #[allow(clippy::too_many_arguments)]
    fn push(
        rendered: &[f32],
        format: StreamFormat,
        device_channels: u16,
        resampler: Option<&mut Resampler>,
        resampled: &mut Vec<f32>,
        mapped: &mut Vec<f32>,
        producer: &mut Producer,
        produced_device_frames: &mut u64,
    ) {
        resampled.clear();
        let at_device_rate: &[f32] = match resampler {
            Some(resampler) => {
                resampler.process(rendered, resampled);
                resampled.as_slice()
            }
            None => rendered,
        };
        mapped.clear();
        map_channels(at_device_rate, format.channels, device_channels, mapped);
        let mut offset = 0;
        while offset < mapped.len() {
            let written = producer.push(&mapped[offset..]);
            if written == 0 {
                std::thread::sleep(Self::IDLE_WAIT);
                continue;
            }
            offset += written;
        }
        *produced_device_frames += (mapped.len() / device_channels.max(1) as usize) as u64;
    }

    fn publish_sample_time(
        graph: &MusicEffectGraph,
        producer: &Producer,
        device_channels: u16,
        device_per_source: f64,
    ) {
        let player = &graph.player;
        let buffered_device_frames = producer.filled() / device_channels.max(1) as usize;
        let rate = f64::from(graph.renderer.varispeed_rate.max(0.01));
        let buffered_source = (buffered_device_frames as f64 / device_per_source * rate) as i64;
        let heard = (player.pulled_frames - buffered_source).max(0);
        player.shared.sample_time.store(heard, Ordering::Release);
        player.shared.has_render_time.store(true, Ordering::Release);
    }

    pub fn enable_manual_rendering_mode(&mut self, format: StreamFormat, maximum_frame_count: usize) {
        self.manual_rendering = Some((format, maximum_frame_count));
        self.manual_rendering_sample_time = 0;
    }

    pub fn disable_manual_rendering_mode(&mut self) {
        self.manual_rendering = None;
    }

    pub fn manual_rendering_format(&self) -> Option<StreamFormat> {
        self.manual_rendering.map(|(format, _)| format)
    }

    pub fn manual_rendering_maximum_frame_count(&self) -> usize {
        self.manual_rendering.map_or(0, |(_, frames)| frames)
    }

    pub fn manual_rendering_sample_time(&self) -> i64 {
        self.manual_rendering_sample_time
    }

    pub fn render_offline(
        &mut self,
        frames: usize,
        buffer: &mut Vec<f32>,
    ) -> AVAudioEngineManualRenderingStatus {
        let Some((format, maximum)) = self.manual_rendering else {
            return AVAudioEngineManualRenderingStatus::CannotDoInCurrentContext;
        };
        let Some(graph) = self.graph.as_mut() else {
            return AVAudioEngineManualRenderingStatus::Error;
        };
        if frames > maximum || graph.format != format {
            return AVAudioEngineManualRenderingStatus::Error;
        }
        buffer.clear();
        graph.render(frames, buffer);
        self.manual_rendering_sample_time += frames as i64;
        AVAudioEngineManualRenderingStatus::Success
    }
}

impl Drop for MusicEffectAudioEngine {
    fn drop(&mut self) {
        self.stop();
    }
}
