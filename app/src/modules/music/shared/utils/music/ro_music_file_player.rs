use std::path::{Path, PathBuf};
use std::time::Duration;

use futures::StreamExt;
use futures::channel::mpsc;
use gpui::{Context, EntityId, Task};
use rise_core::foundation::foundation_date::RiseClock;
use rise_ui::swiftui::repeating_timer::RepeatingTimer;

use crate::core::desktop_platform::rise_app_events::{RiseAppEvents, RiseNotificationName};
use crate::modules::music::shared::utils::music::music_audio_effect_settings::MusicAudioEffectSettings;
use crate::modules::music::shared::utils::music::music_effect_audio_graph::{
    MusicEffectAudioChain, MusicEffectAudioEngine, MusicEffectAudioFile, MusicEffectPlayerNode,
};

pub const RO_MUSIC_FILE_PLAYER_DID_FINISH_PLAYING: RiseNotificationName =
    RiseNotificationName::new("ROMusicFilePlayerDidFinishPlaying");

/// notification payload with the finished object and the success flag
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ROMusicFilePlayerDidFinishPlaying {
    pub object: EntityId,
    pub successfully: bool,
}

const PREFERRED_IO_BUFFER_DURATION: f64 = 0.046;
const STALL_RECOVERY_LIMIT: i64 = 10;
const DUCKED_VOLUME: f32 = 0.02;

/// everything one opened file needs to play, built and started off the main thread
pub struct ROMusicFilePreparedPlayback {
    engine: MusicEffectAudioEngine,
    node: MusicEffectPlayerNode,
    chain: MusicEffectAudioChain,
    file: MusicEffectAudioFile,
    file_url: PathBuf,
    sample_rate: f64,
    prepared_playback_position: f64,
    schedule_start_frame: i64,
    schedule_generation: u64,
    completions: mpsc::UnboundedSender<u64>,
    completion_inbox: mpsc::UnboundedReceiver<u64>,
    output_gain: f32,
    at_full_volume: bool,
}

impl ROMusicFilePreparedPlayback {
    pub fn prepare_file_url(url: &Path, from_time: f64) -> Option<Self> {
        if !url.is_absolute() {
            return None;
        }
        let file = MusicEffectAudioFile::for_reading(url).ok()?;
        let format = file.processing_format();
        let sample_rate = f64::from(format.sample_rate);
        if sample_rate <= 0.0 || file.length() <= 0 {
            return None;
        }
        let mut engine = MusicEffectAudioEngine::new();
        engine.set_preferred_io_buffer_duration(PREFERRED_IO_BUFFER_DURATION);
        let node = MusicEffectPlayerNode::new();
        let settings = MusicAudioEffectSettings::off();
        let mut chain = MusicEffectAudioChain::new(sample_rate);
        chain.attach(&mut engine, &node, format, &settings);
        chain.apply(&settings);
        chain.set_ambience_muted(true);
        engine.prepare();
        let (completions, completion_inbox) = mpsc::unbounded::<u64>();
        let total_frames = file.length();
        let start_frame = Self::frame(from_time.max(0.0), sample_rate).min(total_frames - 1);
        let mut prepared = Self {
            engine,
            node,
            chain,
            file,
            file_url: url.to_path_buf(),
            sample_rate,
            prepared_playback_position: start_frame as f64 / sample_rate,
            schedule_start_frame: start_frame,
            schedule_generation: 0,
            completions,
            completion_inbox,
            output_gain: 1.0,
            at_full_volume: false,
        };
        prepared.schedule(start_frame);
        Some(prepared)
    }

    /// opens the output device on the worker at zero volume
    /// device starts while the node holds its first frame
    /// so the candidate cant finish before it is published
    pub fn start_prepared_silently(&mut self) -> bool {
        self.node.set_volume(0.0);
        self.at_full_volume = false;
        self.engine.start().is_ok()
    }

    pub fn commit_prepared_playback(&mut self) -> bool {
        let position = (self.schedule_start_frame + self.node.player_sample_time().unwrap_or(0)) as f64
            / self.sample_rate;
        if (position - self.prepared_playback_position).abs() > 1.0 / self.sample_rate.max(1.0) {
            return false;
        }
        self.node.set_volume(self.output_gain);
        self.at_full_volume = true;
        self.chain.set_ambience_muted(false);
        self.node.play();
        true
    }

    pub fn reset_prepared_playback_position(&mut self) {
        let frame = Self::frame(self.prepared_playback_position, self.sample_rate);
        self.schedule(frame);
    }

    fn schedule(&mut self, from_frame: i64) {
        self.node.stop();
        self.schedule_generation += 1;
        self.schedule_start_frame = from_frame;
        let generation = self.schedule_generation;
        let completions = self.completions.clone();
        let frame_count = (self.file.length() - from_frame).max(0);
        self.node.schedule_segment(&self.file, from_frame, frame_count, move || {
            let _ = completions.unbounded_send(generation);
        });
    }

    fn frame(seconds: f64, sample_rate: f64) -> i64 {
        (seconds.max(0.0) * sample_rate).round() as i64
    }
}

/// the published player, what the state holds once a candidate was committed
pub struct ROMusicFilePlayer {
    engine: MusicEffectAudioEngine,
    node: MusicEffectPlayerNode,
    chain: Option<MusicEffectAudioChain>,
    file: Option<MusicEffectAudioFile>,
    file_url: Option<PathBuf>,
    total_frames: i64,
    sample_rate: f64,
    schedule_start_frame: i64,
    schedule_generation: u64,
    has_pending_schedule: bool,
    stall_recovery_count: i64,
    fade_timer: Option<RepeatingTimer>,
    completions: mpsc::UnboundedSender<u64>,
    _completion_listener: Task<()>,
    output_gain: f32,
    at_full_volume: bool,
}

impl ROMusicFilePlayer {
    /// publishes a committed candidate, from here on it reports its finish
    pub fn new(prepared: ROMusicFilePreparedPlayback, cx: &mut Context<Self>) -> Self {
        let ROMusicFilePreparedPlayback {
            engine,
            node,
            chain,
            file,
            file_url,
            sample_rate,
            schedule_start_frame,
            schedule_generation,
            completions,
            mut completion_inbox,
            output_gain,
            at_full_volume,
            ..
        } = prepared;
        let completion_listener = cx.spawn(async move |this, cx| {
            while let Some(generation) = completion_inbox.next().await {
                if this
                    .update(cx, |this, cx| this.handle_schedule_completed(generation, cx))
                    .is_err()
                {
                    break;
                }
            }
        });
        Self {
            total_frames: file.length(),
            engine,
            node,
            chain: Some(chain),
            file: Some(file),
            file_url: Some(file_url),
            sample_rate,
            schedule_start_frame,
            schedule_generation,
            has_pending_schedule: true,
            stall_recovery_count: 0,
            fade_timer: None,
            completions,
            _completion_listener: completion_listener,
            output_gain,
            at_full_volume,
        }
    }

    /// app sound level during a call (0 to 1, default 1)
    /// full volume and every ramp are scaled by it
    pub fn output_gain(&self) -> f32 {
        self.output_gain
    }

    /// changing the gain moves a player at full volume right away
    pub fn set_output_gain(&mut self, output_gain: f32) {
        self.output_gain = output_gain.clamp(0.0, 1.0);
        if self.file.is_some() && self.at_full_volume {
            self.fade_timer = None;
            self.node.set_volume(self.output_gain);
        }
    }

    pub fn is_playing(&self) -> bool {
        self.file.is_some() && self.node.is_playing()
    }

    pub fn current_time(&self) -> f64 {
        if self.file.is_none() || self.sample_rate <= 0.0 {
            return 0.0;
        }
        self.current_frame() as f64 / self.sample_rate
    }

    pub fn duration(&self) -> f64 {
        if self.file.is_none() || self.sample_rate <= 0.0 {
            return 0.0;
        }
        let duration = self.total_frames as f64 / self.sample_rate;
        if duration.is_finite() && duration > 0.0 { duration } else { 0.0 }
    }

    pub fn rate(&self) -> f32 {
        if self.is_playing() { 1.0 } else { 0.0 }
    }

    pub fn file_url(&self) -> Option<&PathBuf> {
        self.file_url.as_ref()
    }

    pub fn stall_recovery_count(&self) -> i64 {
        self.stall_recovery_count
    }

    pub fn pause(&mut self) {
        tracing::debug!(target: "riseonly", "ROMusicFilePlayer pause t={:.2}", self.current_time());
        self.node.pause();
    }

    pub fn resume(&mut self) {
        if self.file.is_none() {
            tracing::debug!(target: "riseonly", "ROMusicFilePlayer resume: no player");
            return;
        }
        if self.node.is_playing() {
            tracing::debug!(target: "riseonly", "ROMusicFilePlayer resume: already playing t={:.2}", self.current_time());
            return;
        }
        let ok = self.play();
        tracing::debug!(target: "riseonly", "ROMusicFilePlayer resume: play result={ok} t={:.2}", self.current_time());
    }

    pub fn seek(&mut self, to_time: f64) {
        if self.file.is_none() {
            return;
        }
        tracing::debug!(target: "riseonly", "ROMusicFilePlayer seek {:.2} to {:.2}", self.current_time(), to_time);
        let was_playing = self.node.is_playing();
        let frame = self.frame(to_time);
        if !self.schedule(frame) {
            return;
        }
        if was_playing {
            self.node.play();
        }
    }

    pub fn recover_if_stalled(&mut self) -> bool {
        if self.file.is_none() {
            tracing::debug!(target: "riseonly", "ROMusicFilePlayer recoverIfStalled: no player");
            return false;
        }
        if self.stall_recovery_count >= STALL_RECOVERY_LIMIT {
            tracing::debug!(target: "riseonly", "ROMusicFilePlayer recoverIfStalled: limit reached ({})", self.stall_recovery_count);
            return false;
        }
        self.stall_recovery_count += 1;
        self.node.pause();
        let ok = self.play();
        tracing::debug!(
            target: "riseonly",
            "ROMusicFilePlayer recoverIfStalled #{}: play result={ok} t={:.2}",
            self.stall_recovery_count,
            self.current_time()
        );
        ok
    }

    pub fn stop(&mut self) {
        tracing::debug!(target: "riseonly", "ROMusicFilePlayer stop t={:.2}", self.current_time());
        self.fade_timer = None;
        self.node.stop();
        self.engine.stop();
        self.schedule_generation += 1;
        self.has_pending_schedule = false;
        self.chain = None;
        self.file = None;
        self.file_url = None;
        self.stall_recovery_count = 0;
    }

    /// linear volume ramp (0 to 1, scaled by output gain) over fade_duration seconds
    pub fn set_volume(&mut self, volume: f32, fade_duration: f64, cx: &mut Context<Self>) {
        if self.file.is_none() {
            return;
        }
        let level = volume.clamp(0.0, 1.0);
        self.at_full_volume = level >= 1.0;
        self.ramp_volume(level * self.output_gain, fade_duration.max(0.0), cx);
    }

    pub fn fade_out_and_pause(&mut self, duration: f64, cx: &mut Context<Self>) {
        if self.file.is_none() {
            return;
        }
        let duration = duration.max(0.0);
        self.at_full_volume = false;
        self.ramp_volume(0.0, duration, cx);
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_secs_f64(duration)).await;
            let _ = this.update(cx, |this, _| {
                if this.file.is_some() && !this.at_full_volume && this.node.volume() <= DUCKED_VOLUME {
                    this.node.pause();
                }
            });
        })
        .detach();
    }

    pub fn fade_in_and_resume(&mut self, duration: f64, cx: &mut Context<Self>) {
        if self.file.is_none() {
            return;
        }
        self.fade_timer = None;
        self.node.set_volume(0.0);
        let ok = self.play();
        self.at_full_volume = true;
        self.ramp_volume(self.output_gain, duration.max(0.0), cx);
        tracing::debug!(target: "riseonly", "ROMusicFilePlayer fadeInAndResume play={ok}");
    }

    pub fn restore_full_volume(&mut self) {
        if self.file.is_none() {
            return;
        }
        self.fade_timer = None;
        self.at_full_volume = true;
        self.node.set_volume(self.output_gain);
    }

    /// resumes after a pause
    /// restarts device and schedule if they ended, a finished file replays from where it stands
    fn play(&mut self) -> bool {
        if !self.engine.is_running() && self.engine.start().is_err() {
            return false;
        }
        if !self.has_pending_schedule {
            let frame = self.current_frame();
            if frame >= self.total_frames || !self.schedule(frame) {
                return false;
            }
        }
        if let Some(chain) = self.chain.as_ref() {
            chain.set_ambience_muted(false);
        }
        self.node.play();
        true
    }

    fn current_frame(&self) -> i64 {
        let heard = self.node.player_sample_time().unwrap_or(0);
        (self.schedule_start_frame + heard).clamp(0, self.total_frames.max(0))
    }

    fn frame(&self, seconds: f64) -> i64 {
        (seconds.max(0.0) * self.sample_rate).round() as i64
    }

    fn schedule(&mut self, from_frame: i64) -> bool {
        let Some(file) = self.file.as_ref() else {
            return false;
        };
        let start = from_frame.max(0).min((self.total_frames - 1).max(0));
        let frame_count = (self.total_frames - start).max(0);
        if frame_count <= 0 {
            return false;
        }
        self.node.stop();
        self.schedule_generation += 1;
        self.schedule_start_frame = start;
        self.has_pending_schedule = true;
        let generation = self.schedule_generation;
        let completions = self.completions.clone();
        self.node.schedule_segment(file, start, frame_count, move || {
            let _ = completions.unbounded_send(generation);
        });
        true
    }

    fn handle_schedule_completed(&mut self, generation: u64, cx: &mut Context<Self>) {
        if generation != self.schedule_generation || !self.has_pending_schedule {
            return;
        }
        self.has_pending_schedule = false;
        let reached = self.current_frame();
        let tolerance = (self.sample_rate / 2.0).round() as i64;
        let successfully = reached + tolerance >= self.total_frames;
        self.node.pause();
        tracing::debug!(
            target: "riseonly",
            "ROMusicFilePlayer didFinishPlaying successfully={successfully} t={:.2} dur={:.2}",
            self.current_time(),
            self.duration()
        );
        RiseAppEvents::default_center().post_with(
            RO_MUSIC_FILE_PLAYER_DID_FINISH_PLAYING,
            ROMusicFilePlayerDidFinishPlaying {
                object: cx.entity_id(),
                successfully,
            },
        );
    }

    fn ramp_volume(&mut self, target: f32, duration: f64, cx: &mut Context<Self>) {
        self.fade_timer = None;
        if duration <= 0.01 {
            self.node.set_volume(target);
            return;
        }
        let start = self.node.volume();
        let started_at = RiseClock::monotonic();
        self.fade_timer = Some(RepeatingTimer::scheduled_timer(
            Duration::from_secs_f64(1.0 / 60.0),
            true,
            cx,
            move |this, _| {
                let elapsed = RiseClock::monotonic().saturating_sub(started_at).as_secs_f64();
                let progress = (elapsed / duration).min(1.0);
                this.node.set_volume(start + (target - start) * progress as f32);
                if progress >= 1.0 {
                    this.fade_timer = None;
                }
            },
        ));
    }
}
