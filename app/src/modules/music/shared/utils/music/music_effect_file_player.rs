use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::time::Duration;

use futures::StreamExt;
use futures::channel::mpsc;
use gpui::{App, Context, EntityId, Task};
use rise_core::foundation::foundation_date::RiseClock;
use rise_engine::CancellationError;
use rise_ui::swiftui::repeating_timer::RepeatingTimer;

use crate::core::desktop_platform::av_audio_session::AVAudioSession;
use crate::core::desktop_platform::rise_app_events::{
    RiseAppEvents, RiseNotificationName, RiseSubscription,
};
use crate::core::lib::rise_audio_playback_work::{RiseAudioPlaybackWork, RisePlaybackAudioSession};
use crate::core::lib::rise_call_audio_ownership::RiseCallAudioOwnership;
use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::core::lib::sai::sai::Sai;
use crate::modules::music::shared::utils::music::music_audio_effect_settings::{
    MusicAudioEffectDSP, MusicAudioEffectKind, MusicAudioEffectSettings, MusicEffectCanon,
    MusicEffectScaleId,
};
use crate::modules::music::shared::utils::music::music_effect_audio_graph::{
    MusicEffectAudioChain, MusicEffectAudioEngine, MusicEffectAudioEngineError, MusicEffectAudioFile,
    MusicEffectAudioFileError, MusicEffectPlayerNode,
};

enum MusicEffectLog {}

impl MusicEffectLog {
    fn kind() -> SaiLogKind {
        SaiLogKind::Custom("player".to_owned())
    }

    fn info(message: String, cx: &mut App) {
        Sai::log("🎛️ Effect", move || message, Self::kind(), cx);
    }

    fn fail(message: String, cx: &mut App) {
        Sai::log("🎛️ Effect ❌", move || message, Self::kind(), cx);
    }
}

pub const MUSIC_EFFECT_FILE_PLAYER_DID_FINISH_PLAYING: RiseNotificationName =
    RiseNotificationName::new("MusicEffectFilePlayerDidFinishPlaying");

/// object and the successfully flag of the notification
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MusicEffectFilePlayerDidFinishPlaying {
    pub object: EntityId,
    pub successfully: bool,
}

pub enum MusicEffectPlaybackLimits {}

impl MusicEffectPlaybackLimits {
    fn rate_scale() -> &'static rise_music::EffectScale {
        MusicEffectCanon::scale(MusicEffectScaleId::Rate)
    }

    fn speed_up_scale() -> &'static rise_music::EffectScale {
        MusicEffectCanon::scale(MusicEffectScaleId::SpeedUp)
    }

    fn intensity_scale() -> &'static rise_music::EffectScale {
        MusicEffectCanon::scale(MusicEffectScaleId::Intensity)
    }

    pub fn minimum_rate() -> f64 {
        MusicEffectCanon::unit(Self::rate_scale().min_milli)
    }

    pub fn maximum_rate() -> f64 {
        MusicEffectCanon::unit(Self::rate_scale().max_milli)
    }

    pub fn default_rate() -> f64 {
        MusicEffectCanon::unit(Self::rate_scale().default_milli)
    }

    pub fn rate_step() -> f64 {
        MusicEffectCanon::unit(Self::rate_scale().step_milli)
    }

    pub fn minimum_speed_up() -> f64 {
        MusicEffectCanon::unit(Self::speed_up_scale().min_milli)
    }

    pub fn maximum_speed_up() -> f64 {
        MusicEffectCanon::unit(Self::speed_up_scale().max_milli)
    }

    pub fn default_speed_up() -> f64 {
        MusicEffectCanon::unit(Self::speed_up_scale().default_milli)
    }

    pub fn minimum_intensity() -> f64 {
        MusicEffectCanon::unit(Self::intensity_scale().min_milli)
    }

    pub fn maximum_intensity() -> f64 {
        MusicEffectCanon::unit(Self::intensity_scale().max_milli)
    }

    pub fn default_intensity() -> f64 {
        MusicEffectCanon::unit(Self::intensity_scale().default_milli)
    }

    pub fn intensity_step() -> f64 {
        MusicEffectCanon::unit(Self::intensity_scale().step_milli)
    }

    pub fn intensity_breakpoints() -> Vec<f64> {
        Self::intensity_scale()
            .breakpoints_milli
            .iter()
            .map(|milli| MusicEffectCanon::unit(*milli))
            .collect()
    }

    /// snap targets for the tempo slider
    /// the dense 0.75 to 0.9 span is where the effect lives so it gets finer stops than the extremes
    pub fn breakpoints() -> Vec<f64> {
        Self::rate_scale()
            .breakpoints_milli
            .iter()
            .map(|milli| MusicEffectCanon::unit(*milli))
            .collect()
    }

    pub fn clamp_rate(value: f64) -> f64 {
        Self::maximum_rate().min(Self::minimum_rate().max(value))
    }

    pub fn quantize_rate(value: f64) -> f64 {
        MusicEffectCanon::unit(MusicEffectCanon::quantize_milli(
            MusicEffectCanon::milli(value),
            Self::rate_scale(),
        ))
    }

    pub fn clamp_intensity(value: f64) -> f64 {
        Self::maximum_intensity().min(Self::minimum_intensity().max(value))
    }

    pub fn quantize_intensity(value: f64) -> f64 {
        MusicEffectCanon::unit(MusicEffectCanon::quantize_milli(
            MusicEffectCanon::milli(value),
            Self::intensity_scale(),
        ))
    }

    pub fn rate_milli(value: f64) -> i64 {
        MusicEffectCanon::milli(Self::clamp_rate(value))
    }

    /// converts a milli rate to a plain f64 rate
    pub fn rate(from_milli: i64) -> f64 {
        Self::clamp_rate(MusicEffectCanon::unit(from_milli))
    }

}

/// why a prepare gave no graph, only the log reads it
#[derive(Debug)]
enum MusicEffectPrepareFailure {
    Cancelled,
    FileReadCorruptFile,
    File(MusicEffectAudioFileError),
    Engine(MusicEffectAudioEngineError),
}

impl MusicEffectPrepareFailure {
    fn description(&self) -> String {
        match self {
            Self::Cancelled => "cancelled".to_owned(),
            Self::FileReadCorruptFile => "file read corrupt file".to_owned(),
            Self::File(error) => format!("{error:?}"),
            Self::Engine(error) => format!("{error:?}"),
        }
    }
}

impl From<CancellationError> for MusicEffectPrepareFailure {
    fn from(_: CancellationError) -> Self {
        Self::Cancelled
    }
}

/// graph built on the audio worker, handed to the player whole
struct MusicEffectPreparedGraph {
    file: MusicEffectAudioFile,
    engine: MusicEffectAudioEngine,
    node: MusicEffectPlayerNode,
    chain: MusicEffectAudioChain,
}

/// plays a local file through the effect graph
/// slowed with pitch drop and reverb
/// optional vinyl crackle and low-pass
pub struct MusicEffectFilePlayer {
    engine: MusicEffectAudioEngine,
    player_node: MusicEffectPlayerNode,
    chain: Option<MusicEffectAudioChain>,

    file: Option<MusicEffectAudioFile>,
    file_format: Option<rise_media::audio::format::StreamFormat>,
    total_frames: i64,
    file_sample_rate: f64,

    schedule_start_frame: i64,
    last_known_frame: Cell<i64>,
    schedule_generation: u64,
    configuration_observer: Option<RiseSubscription>,
    configuration_task: Option<Task<()>>,
    fade_timer: Option<RepeatingTimer>,
    reverb_ramp_timer: Option<RepeatingTimer>,
    spin_timer: Option<RepeatingTimer>,
    did_reach_end: bool,
    has_pending_schedule: bool,
    playback_requested: bool,

    file_url: Option<PathBuf>,
    stall_recovery_count: i64,
    settings: MusicAudioEffectSettings,
    output_gain: f32,

    completions: mpsc::UnboundedSender<u64>,
    configuration_changes: mpsc::UnboundedSender<()>,
    _completion_listener: Task<()>,
    _configuration_listener: Task<()>,
}

impl MusicEffectFilePlayer {
    const PREFERRED_IO_BUFFER_DURATION: f64 = 0.046;

    pub fn new(cx: &mut Context<Self>) -> Self {
        let (completions, mut completion_inbox) = mpsc::unbounded::<u64>();
        let (configuration_changes, mut configuration_inbox) = mpsc::unbounded::<()>();
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
        let configuration_listener = cx.spawn(async move |this, cx| {
            while configuration_inbox.next().await.is_some() {
                if this
                    .update(cx, |this, cx| this.handle_configuration_change(cx))
                    .is_err()
                {
                    break;
                }
            }
        });
        let mut engine = MusicEffectAudioEngine::new();
        engine.set_preferred_io_buffer_duration(Self::PREFERRED_IO_BUFFER_DURATION);
        Self {
            engine,
            player_node: MusicEffectPlayerNode::new(),
            chain: None,
            file: None,
            file_format: None,
            total_frames: 0,
            file_sample_rate: 44_100.0,
            schedule_start_frame: 0,
            last_known_frame: Cell::new(0),
            schedule_generation: 0,
            configuration_observer: None,
            configuration_task: None,
            fade_timer: None,
            reverb_ramp_timer: None,
            spin_timer: None,
            did_reach_end: false,
            has_pending_schedule: false,
            playback_requested: false,
            file_url: None,
            stall_recovery_count: 0,
            settings: MusicAudioEffectSettings::off(),
            output_gain: 1.0,
            completions,
            configuration_changes,
            _completion_listener: completion_listener,
            _configuration_listener: configuration_listener,
        }
    }

    pub fn file_url(&self) -> Option<&PathBuf> {
        self.file_url.as_ref()
    }

    pub fn stall_recovery_count(&self) -> i64 {
        self.stall_recovery_count
    }

    pub fn settings(&self) -> &MusicAudioEffectSettings {
        &self.settings
    }

    pub fn is_playing(&self) -> bool {
        self.engine.is_running() && self.player_node.is_playing()
    }

    pub fn rate(&self) -> f32 {
        if self.is_playing() {
            self.settings.playback_rate() as f32
        } else {
            0.0
        }
    }

    pub fn effect_rate(&self) -> f64 {
        self.settings.playback_rate()
    }

    pub fn is_effect_enabled(&self) -> bool {
        self.settings.is_any_enabled()
    }

    pub fn duration(&self) -> f64 {
        if self.file_sample_rate <= 0.0 || self.total_frames <= 0 {
            return 0.0;
        }
        self.total_frames as f64 / self.file_sample_rate
    }

    pub fn current_time(&self) -> f64 {
        if self.file_sample_rate <= 0.0 {
            return 0.0;
        }
        self.current_frame().max(0).min(self.total_frames.max(0)) as f64 / self.file_sample_rate
    }

    fn current_frame(&self) -> i64 {
        if !self.player_node.is_playing() {
            return self.last_known_frame.get();
        }
        let Some(sample_time) = self.player_node.player_sample_time() else {
            return self.last_known_frame.get();
        };
        self.last_known_frame.set(self.schedule_start_frame + sample_time);
        self.last_known_frame.get()
    }

    pub fn prepare_file_url(
        &mut self,
        url: PathBuf,
        settings: MusicAudioEffectSettings,
        expected_duration: f64,
        cx: &mut Context<Self>,
    ) -> Task<bool> {
        if !url.is_absolute() {
            return Task::ready(false);
        }
        self.stop();
        let worker_url = url.clone();
        let worker_settings = settings.clone();
        cx.spawn(async move |this, cx| {
            let prepared = RiseAudioPlaybackWork::perform(
                |prepared: MusicEffectPreparedGraph| drop(prepared),
                move |check_cancellation| {
                    Self::build_graph(&worker_url, &worker_settings, expected_duration, check_cancellation)
                },
            )
            .await;
            match prepared {
                Ok(prepared) => this
                    .update(cx, |this, cx| {
                        let format = prepared.file.processing_format();
                        this.total_frames = prepared.file.length();
                        this.file = Some(prepared.file);
                        this.engine = prepared.engine;
                        this.engine.set_main_mixer_output_volume(this.output_gain);
                        this.player_node = prepared.node;
                        this.chain = Some(prepared.chain);
                        this.file_format = Some(format);
                        this.file_sample_rate = f64::from(format.sample_rate);
                        this.file_url = Some(url);
                        this.settings = settings;
                        this.install_configuration_observer_if_needed();
                        this.update_spin_timer(cx);
                        true
                    })
                    .unwrap_or(false),
                Err(MusicEffectPrepareFailure::Cancelled) => false,
                Err(error) => {
                    cx.update(|cx| {
                        MusicEffectLog::fail(format!("prepare failed: {}", error.description()), cx);
                    });
                    false
                }
            }
        })
    }

    fn build_graph(
        url: &Path,
        settings: &MusicAudioEffectSettings,
        expected_duration: f64,
        check_cancellation: &dyn Fn() -> Result<(), CancellationError>,
    ) -> Result<MusicEffectPreparedGraph, MusicEffectPrepareFailure> {
        let opened = MusicEffectAudioFile::for_reading(url).map_err(MusicEffectPrepareFailure::File)?;
        let format = opened.processing_format();
        let sample_rate = f64::from(format.sample_rate);
        if sample_rate <= 0.0
            || opened.length() <= 0
            || !(expected_duration <= 1.0
                || opened.length() as f64 / sample_rate >= expected_duration * 0.5)
        {
            return Err(MusicEffectPrepareFailure::FileReadCorruptFile);
        }
        check_cancellation()?;
        let mut engine = MusicEffectAudioEngine::new();
        engine.set_preferred_io_buffer_duration(Self::PREFERRED_IO_BUFFER_DURATION);
        let node = MusicEffectPlayerNode::new();
        let mut chain = MusicEffectAudioChain::new(sample_rate);
        chain.attach(&mut engine, &node, format, settings);
        let _ = chain.update_reverb_preset(settings);
        chain.apply(settings);
        chain.set_reverb_mix(MusicAudioEffectDSP::reverb_wet_dry_mix(settings));
        chain.set_ambience_muted(true);
        node.set_volume(0.0);
        engine.prepare();
        check_cancellation()?;
        if RiseCallAudioOwnership::blocks_music() {
            return Err(MusicEffectPrepareFailure::Cancelled);
        }
        engine.start().map_err(MusicEffectPrepareFailure::Engine)?;
        Ok(MusicEffectPreparedGraph {
            file: opened,
            engine,
            node,
            chain,
        })
    }

    pub fn play_prepared(&mut self, from_time: f64) -> bool {
        if self.file.is_none()
            || RiseCallAudioOwnership::blocks_music()
            || !self.schedule(self.frame(from_time))
        {
            return false;
        }
        self.player_node.set_volume(1.0);
        if let Some(chain) = self.chain.as_ref() {
            chain.set_ambience_muted(false);
        }
        self.playback_requested = true;
        self.player_node.play();
        true
    }

    pub fn pause(&mut self) {
        self.configuration_task = None;
        self.playback_requested = false;
        self.cancel_fade_timer();
        let _ = self.current_frame();
        self.player_node.pause();
        if let Some(chain) = self.chain.as_ref() {
            chain.set_ambience_muted(true);
        }
    }

    pub fn resume(&mut self, cx: &mut Context<Self>) {
        if self.file.is_none() || RiseCallAudioOwnership::blocks_music() {
            return;
        }
        if !self.engine.is_running()
            && let Err(error) = self.engine.start()
        {
            MusicEffectLog::fail(format!("engine restart failed: {error:?}"), cx);
            return;
        }
        if !self.has_pending_schedule {
            let last_known = self.last_known_frame.get();
            if last_known >= self.total_frames || !self.schedule(last_known) {
                return;
            }
        }
        if let Some(chain) = self.chain.as_ref() {
            chain.set_ambience_muted(false);
        }
        self.playback_requested = true;
        self.player_node.play();
    }

    pub fn seek(&mut self, to_time: f64) {
        if self.file.is_none() {
            return;
        }
        let was_playing = self.player_node.is_playing();
        if !self.schedule(self.frame(to_time)) {
            return;
        }
        if was_playing {
            self.player_node.play();
        }
    }

    pub fn recover_if_stalled(&mut self, cx: &mut Context<Self>) -> bool {
        if self.file.is_none() || self.stall_recovery_count >= 10 {
            return false;
        }
        self.stall_recovery_count += 1;
        let resume_frame = self.last_known_frame.get();
        if RiseCallAudioOwnership::blocks_music() {
            return false;
        }
        if !self.engine.is_running()
            && let Err(error) = self.engine.start()
        {
            MusicEffectLog::fail(format!("recover engine start failed: {error:?}"), cx);
            return false;
        }
        if !self.schedule(resume_frame) {
            return false;
        }
        self.playback_requested = true;
        self.player_node.play();
        let message = format!(
            "recovered #{} at {:.1}s",
            self.stall_recovery_count,
            self.current_time()
        );
        MusicEffectLog::info(message, cx);
        true
    }

    pub fn stop(&mut self) {
        self.configuration_task = None;
        self.cancel_fade_timer();
        self.cancel_reverb_ramp_timer();
        self.cancel_spin_timer();
        self.teardown();
    }

    pub fn requires_rebuild(&self, new_settings: &MusicAudioEffectSettings) -> bool {
        self.chain
            .as_ref()
            .is_some_and(|chain| chain.requires_rebuild(new_settings))
    }

    /// true while the graph runs the quieter spatialised setup, so the caller expects a bigger level change
    pub fn is_spatialised(&self) -> bool {
        self.chain.as_ref().is_some_and(|chain| chain.spatialised())
    }

    /// rate changes glide on a live graph with no click
    /// reverb cant, so a changed wet/dry mix is ramped
    pub fn apply(&mut self, new_settings: MusicAudioEffectSettings, cx: &mut Context<Self>) {
        if new_settings == self.settings {
            return;
        }
        let previous_mix = MusicAudioEffectDSP::reverb_wet_dry_mix(&self.settings);
        let next_mix = MusicAudioEffectDSP::reverb_wet_dry_mix(&new_settings);
        self.settings = new_settings;
        self.apply_effect_parameters((previous_mix - next_mix).abs() > 0.5, cx);
    }

    /// app sound level during a call, applied on the mixer so duck and start fades still work on top
    pub fn output_gain(&self) -> f32 {
        self.output_gain
    }

    pub fn set_output_gain(&mut self, output_gain: f32) {
        self.output_gain = output_gain;
        self.engine.set_main_mixer_output_volume(output_gain);
    }

    pub fn set_volume(&mut self, volume: f32, fade_duration: f64, cx: &mut Context<Self>) {
        self.ramp_volume(volume.clamp(0.0, 1.0), fade_duration.max(0.0), None, cx);
    }

    pub fn fade_out_and_pause(&mut self, duration: f64, cx: &mut Context<Self>) {
        self.ramp_volume(
            0.0,
            duration.max(0.0),
            Some(Box::new(|this: &mut Self, _: &mut Context<Self>| {
                if this.player_node.volume() <= 0.02 {
                    this.pause();
                }
            })),
            cx,
        );
    }

    pub fn fade_in_and_resume(&mut self, duration: f64, cx: &mut Context<Self>) {
        self.player_node.set_volume(0.0);
        self.resume(cx);
        self.ramp_volume(1.0, duration.max(0.0), None, cx);
    }

    pub fn restore_full_volume(&mut self) {
        self.cancel_fade_timer();
        self.player_node.set_volume(1.0);
    }

    fn attach_nodes(&mut self, format: rise_media::audio::format::StreamFormat) {
        let mut chain = MusicEffectAudioChain::new(f64::from(format.sample_rate));
        chain.attach(&mut self.engine, &self.player_node, format, &self.settings);
        self.chain = Some(chain);
        self.engine.prepare();
    }

    fn apply_effect_parameters(&mut self, ramp_reverb: bool, cx: &mut Context<Self>) {
        let Some(chain) = self.chain.as_mut() else {
            return;
        };
        let did_swap_preset = chain.update_reverb_preset(&self.settings);
        chain.apply(&self.settings);
        self.update_spin_timer(cx);
        let target_mix = MusicAudioEffectDSP::reverb_wet_dry_mix(&self.settings);
        if !(ramp_reverb || did_swap_preset) {
            self.cancel_reverb_ramp_timer();
            if let Some(chain) = self.chain.as_ref() {
                chain.set_reverb_mix(target_mix);
            }
            return;
        }
        self.ramp_reverb_mix(target_mix, cx);
    }

    /// orbit runs from a timer not the render thread, so pan is only touched on the graph thread
    fn update_spin_timer(&mut self, cx: &mut Context<Self>) {
        let needs_spin = self.settings.is_enabled(MusicAudioEffectKind::Spin);
        if !needs_spin {
            self.spin_timer = None;
            if let Some(chain) = self.chain.as_mut() {
                chain.advance_spin(0.0, &self.settings);
            }
            return;
        }
        if self.spin_timer.is_some() {
            return;
        }
        let interval = 1.0 / 30.0;
        self.spin_timer = Some(RepeatingTimer::scheduled_timer(
            Duration::from_secs_f64(interval),
            true,
            cx,
            move |this, _| {
                if !this.player_node.is_playing() {
                    return;
                }
                if let Some(chain) = this.chain.as_mut() {
                    chain.advance_spin(interval, &this.settings);
                }
            },
        ));
    }

    fn schedule(&mut self, from_frame: i64) -> bool {
        let Some(file) = self.file.as_ref() else {
            return false;
        };
        let clamped_start = from_frame.max(0).min((self.total_frames - 1).max(0));
        let frame_count = (self.total_frames - clamped_start).max(0);
        if frame_count <= 0 {
            return false;
        }

        self.player_node.stop();
        self.schedule_generation += 1;
        self.schedule_start_frame = clamped_start;
        self.last_known_frame.set(clamped_start);
        self.did_reach_end = false;
        self.has_pending_schedule = true;
        let generation = self.schedule_generation;
        let completions = self.completions.clone();
        self.player_node
            .schedule_segment(file, clamped_start, frame_count, move || {
                let _ = completions.unbounded_send(generation);
            });
        true
    }

    fn handle_schedule_completed(&mut self, generation: u64, cx: &mut Context<Self>) {
        if generation != self.schedule_generation || self.did_reach_end {
            return;
        }
        self.did_reach_end = true;
        self.playback_requested = false;
        self.has_pending_schedule = false;
        self.last_known_frame.set(self.total_frames);
        if let Some(chain) = self.chain.as_ref() {
            chain.set_ambience_muted(true);
        }
        RiseAppEvents::default_center().post_with(
            MUSIC_EFFECT_FILE_PLAYER_DID_FINISH_PLAYING,
            MusicEffectFilePlayerDidFinishPlaying {
                object: cx.entity_id(),
                successfully: true,
            },
        );
    }

    fn install_configuration_observer_if_needed(&mut self) {
        if self.configuration_observer.is_some() {
            return;
        }
        let changes = self.configuration_changes.clone();
        self.configuration_observer = Some(RiseAppEvents::default_center().observe(
            AVAudioSession::ROUTE_CHANGE_NOTIFICATION,
            move |_| {
                let _ = changes.unbounded_send(());
            },
        ));
    }

    fn handle_configuration_change(&mut self, cx: &mut Context<Self>) {
        let Some(file_format) = self.file_format else {
            return;
        };
        if self.file.is_none() {
            return;
        }
        let resume_frame = self.last_known_frame.get();
        let was_playing = self.playback_requested;
        MusicEffectLog::info("configuration change → rebuilding graph".to_owned(), cx);
        self.schedule_generation += 1;
        self.player_node.stop();
        self.engine.stop();
        self.attach_nodes(file_format);
        self.apply_effect_parameters(false, cx);
        if let Some(chain) = self.chain.as_ref() {
            chain.set_ambience_muted(!was_playing);
        }
        if !self.schedule(resume_frame) || !was_playing {
            return;
        }
        let generation = self.schedule_generation;
        self.configuration_task = Some(cx.spawn(async move |this, cx| {
            if RiseCallAudioOwnership::blocks_music()
                || !RisePlaybackAudioSession::activate().await
                || RiseCallAudioOwnership::blocks_music()
            {
                return;
            }
            let _ = this.update(cx, |this, cx| {
                if this.schedule_generation != generation || !this.playback_requested {
                    return;
                }
                match this.engine.start() {
                    Ok(()) => this.player_node.play(),
                    Err(error) => MusicEffectLog::fail(
                        format!("restart after configuration change failed: {error:?}"),
                        cx,
                    ),
                }
            });
        }));
    }

    fn teardown(&mut self) {
        self.playback_requested = false;
        self.cancel_fade_timer();
        self.cancel_reverb_ramp_timer();
        self.cancel_spin_timer();
        self.schedule_generation += 1;
        if let Some(chain) = self.chain.as_ref() {
            chain.set_ambience_muted(true);
        }
        self.player_node.stop();
        if self.engine.is_running() {
            self.engine.stop();
        }
        self.chain = None;
        self.file = None;
        self.file_format = None;
        self.file_url = None;
        self.total_frames = 0;
        self.schedule_start_frame = 0;
        self.last_known_frame.set(0);
        self.did_reach_end = false;
        self.has_pending_schedule = false;
        self.stall_recovery_count = 0;
    }

    fn frame(&self, seconds: f64) -> i64 {
        (seconds.max(0.0) * self.file_sample_rate).round() as i64
    }

    fn ramp_volume(
        &mut self,
        target: f32,
        duration: f64,
        completion: Option<Box<dyn FnOnce(&mut Self, &mut Context<Self>)>>,
        cx: &mut Context<Self>,
    ) {
        self.cancel_fade_timer();
        if duration <= 0.01 {
            self.player_node.set_volume(target);
            if let Some(completion) = completion {
                completion(self, cx);
            }
            return;
        }
        let start = self.player_node.volume();
        let started_at = RiseClock::monotonic();
        let mut completion = completion;
        self.fade_timer = Some(RepeatingTimer::scheduled_timer(
            Duration::from_secs_f64(1.0 / 60.0),
            true,
            cx,
            move |this, cx| {
                let elapsed = RiseClock::monotonic().saturating_sub(started_at).as_secs_f64();
                let progress = (elapsed / duration).min(1.0);
                this.player_node
                    .set_volume(start + (target - start) * progress as f32);
                if progress < 1.0 {
                    return;
                }
                this.fade_timer = None;
                if let Some(completion) = completion.take() {
                    completion(this, cx);
                }
            },
        ));
    }

    fn ramp_reverb_mix(&mut self, target: f32, cx: &mut Context<Self>) {
        self.cancel_reverb_ramp_timer();
        let Some(chain) = self.chain.as_ref() else {
            return;
        };
        let start = chain.reverb_wet_dry_mix();
        if (target - start).abs() <= 0.5 {
            chain.set_reverb_mix(target);
            return;
        }
        let duration = 0.45;
        let started_at = RiseClock::monotonic();
        self.reverb_ramp_timer = Some(RepeatingTimer::scheduled_timer(
            Duration::from_secs_f64(1.0 / 60.0),
            true,
            cx,
            move |this, _| {
                let Some(chain) = this.chain.as_ref() else {
                    this.reverb_ramp_timer = None;
                    return;
                };
                let elapsed = RiseClock::monotonic().saturating_sub(started_at).as_secs_f64();
                let progress = (elapsed / duration).min(1.0);
                chain.set_reverb_mix(start + (target - start) * progress as f32);
                if progress >= 1.0 {
                    this.reverb_ramp_timer = None;
                }
            },
        ));
    }

    fn cancel_fade_timer(&mut self) {
        self.fade_timer = None;
    }

    fn cancel_reverb_ramp_timer(&mut self) {
        self.reverb_ramp_timer = None;
    }

    fn cancel_spin_timer(&mut self) {
        self.spin_timer = None;
    }
}
