use std::path::{Path, PathBuf};
use std::time::Duration;

use futures::channel::oneshot;
use gpui::{Context, Pixels, Size, Task};
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_media::audio_capture::{AacSettings, AudioRecorder};
use rise_platform::capture_permission::{self, CaptureAccess, CaptureKind};
use rise_platform::file_manager::FileManager;
use rise_platform::haptics::{ImpactStyle, impact};
use rise_platform::paths::temporary_directory;

use crate::core::lib::rise_call_audio_ownership::RiseCallAudioOwnership;
use crate::modules::chat::shared::chat_music_hooks::ChatMusicHooks;
use crate::modules::chat::shared::utils::voice::voice_message_metadata::VoiceMessageMetadata;
use crate::modules::music::shared::utils::music::music_external_audio_owner::MusicExternalAudioOwner;

const METER_INTERVAL: Duration = Duration::from_millis(50);
const PULSE_INTERVAL: Duration = Duration::from_nanos(1_000_000_000 / 30);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum VoiceMessageRecorderPhase {
    Idle,
    Arming,
    Recording { is_locked: bool },
    Paused { is_locked: bool },
}

pub struct VoiceMessageRecorder {
    phase: VoiceMessageRecorderPhase,
    elapsed_milliseconds: i64,
    waveform_peaks: Vec<f32>,
    drag_translation: Size<Pixels>,
    shows_cancel_hint: bool,
    shows_lock_hint: bool,
    pulse_phase: f64,

    audio_recorder: Option<AudioRecorder>,
    meter_timer: Option<Task<()>>,
    pulse_timer: Option<Task<()>>,
    recording_started_at: Option<Duration>,
    accumulated_milliseconds: i64,
    output_url: Option<PathBuf>,
    start_generation: i64,
    session_prepared: bool,
    access_was_denied: bool,
}

impl VoiceMessageRecorder {
    pub const MIN_DURATION_MILLISECONDS: i64 = 1000;
    pub const MAX_DURATION_MILLISECONDS: i64 = 5 * 60 * 1000;
    pub const LOCK_SWIPE_THRESHOLD: f32 = 56.0;
    pub const CANCEL_SWIPE_THRESHOLD: f32 = 72.0;

    pub fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            phase: VoiceMessageRecorderPhase::Idle,
            elapsed_milliseconds: 0,
            waveform_peaks: Vec::new(),
            drag_translation: Size::default(),
            shows_cancel_hint: false,
            shows_lock_hint: false,
            pulse_phase: 0.0,
            audio_recorder: None,
            meter_timer: None,
            pulse_timer: None,
            recording_started_at: None,
            accumulated_milliseconds: 0,
            output_url: None,
            start_generation: 0,
            session_prepared: false,
            access_was_denied: false,
        }
    }

    pub fn phase(&self) -> VoiceMessageRecorderPhase {
        self.phase
    }

    pub fn elapsed_milliseconds(&self) -> i64 {
        self.elapsed_milliseconds
    }

    pub fn waveform_peaks(&self) -> &[f32] {
        &self.waveform_peaks
    }

    pub fn drag_translation(&self) -> Size<Pixels> {
        self.drag_translation
    }

    pub fn shows_cancel_hint(&self) -> bool {
        self.shows_cancel_hint
    }

    pub fn shows_lock_hint(&self) -> bool {
        self.shows_lock_hint
    }

    pub fn pulse_phase(&self) -> f64 {
        self.pulse_phase
    }

    pub fn set_pulse_phase(&mut self, pulse_phase: f64, cx: &mut Context<Self>) {
        self.pulse_phase = pulse_phase;
        cx.notify();
    }

    pub fn access_was_denied(&self) -> bool {
        self.access_was_denied
    }

    pub fn is_active(&self) -> bool {
        match self.phase {
            VoiceMessageRecorderPhase::Idle => false,
            VoiceMessageRecorderPhase::Arming
            | VoiceMessageRecorderPhase::Recording { .. }
            | VoiceMessageRecorderPhase::Paused { .. } => true,
        }
    }

    pub fn is_locked(&self) -> bool {
        match self.phase {
            VoiceMessageRecorderPhase::Recording { is_locked } | VoiceMessageRecorderPhase::Paused { is_locked } => {
                is_locked
            }
            VoiceMessageRecorderPhase::Arming | VoiceMessageRecorderPhase::Idle => false,
        }
    }

    pub fn request_permission_if_needed(&mut self, cx: &mut Context<Self>) -> Task<bool> {
        self.access_was_denied = false;
        match capture_permission::status(CaptureKind::Microphone) {
            Ok(CaptureAccess::Granted) => Task::ready(true),
            Ok(CaptureAccess::Denied | CaptureAccess::Restricted) => {
                self.access_was_denied = true;
                Task::ready(false)
            }
            Ok(CaptureAccess::NotDetermined) => cx.spawn(async move |_, _| {
                capture_permission::request(CaptureKind::Microphone).await.unwrap_or(false)
            }),
            Err(error) => {
                tracing::debug!(%error, "microphone access is unreadable on this host");
                Task::ready(false)
            }
        }
    }

    pub fn begin_hold_recording(&mut self, cx: &mut Context<Self>) -> Task<bool> {
        match self.phase {
            VoiceMessageRecorderPhase::Arming
            | VoiceMessageRecorderPhase::Recording { .. }
            | VoiceMessageRecorderPhase::Paused { .. } => return Task::ready(true),
            VoiceMessageRecorderPhase::Idle => {}
        }
        if capture_permission::status(CaptureKind::Microphone) == Ok(CaptureAccess::NotDetermined) {
            let asked = self.request_permission_if_needed(cx);
            return cx.spawn(async move |_, _| {
                asked.await;
                false
            });
        }
        self.start_generation = self.start_generation.wrapping_add(1);
        let generation = self.start_generation;
        self.phase = VoiceMessageRecorderPhase::Arming;
        cx.notify();

        let granted = self.request_permission_if_needed(cx);
        let (answer, answered) = oneshot::channel::<bool>();
        cx.spawn(async move |this, cx| {
            let granted = granted.await;
            let start = this
                .update(cx, |recorder, cx| {
                    if generation != recorder.start_generation {
                        return None;
                    }
                    if !granted {
                        recorder.reset_state(cx);
                        return None;
                    }
                    recorder.prepare_audio_session_if_needed(cx);
                    if generation != recorder.start_generation {
                        return None;
                    }
                    Some(recorder.start_recorder(false, generation, cx))
                })
                .ok()
                .flatten();
            let started = match start {
                Some(start) => start.await,
                None => false,
            };
            let current = this.read_with(cx, |recorder, _| generation == recorder.start_generation).unwrap_or(false);
            answer.send(current && started).ok();
        })
        .detach();
        cx.spawn(async move |_, _| answered.await.unwrap_or(false))
    }

    pub fn lock_recording(&mut self, cx: &mut Context<Self>) {
        let VoiceMessageRecorderPhase::Recording { .. } = self.phase else {
            return;
        };
        self.phase = VoiceMessageRecorderPhase::Recording { is_locked: true };
        self.drag_translation = Size::default();
        self.shows_cancel_hint = false;
        self.shows_lock_hint = false;
        impact(ImpactStyle::Medium);
        cx.notify();
    }

    pub fn update_drag(&mut self, translation: Size<Pixels>, cx: &mut Context<Self>) {
        if let VoiceMessageRecorderPhase::Paused { .. } = self.phase {
            return;
        }
        self.drag_translation = translation;
        let width = f32::from(translation.width);
        let height = f32::from(translation.height);
        self.shows_cancel_hint = width < -Self::CANCEL_SWIPE_THRESHOLD * 0.45;

        if !self.is_locked() {
            self.shows_lock_hint = height < -Self::LOCK_SWIPE_THRESHOLD * 0.45;
            if matches!(self.phase, VoiceMessageRecorderPhase::Recording { .. }) && height < -Self::LOCK_SWIPE_THRESHOLD {
                self.lock_recording(cx);
            }
        }
        cx.notify();
    }

    pub fn end_hold(&mut self, send_if_valid: bool, cx: &mut Context<Self>) -> VoiceRecordingFinishResult {
        if self.phase == VoiceMessageRecorderPhase::Arming {
            self.cancel_recording(cx);
            return VoiceRecordingFinishResult::Cancelled;
        }
        let VoiceMessageRecorderPhase::Recording { is_locked } = self.phase else {
            return VoiceRecordingFinishResult::Ignored;
        };
        if is_locked {
            return VoiceRecordingFinishResult::Ignored;
        }
        if f32::from(self.drag_translation.width) < -Self::CANCEL_SWIPE_THRESHOLD {
            self.cancel_recording(cx);
            return VoiceRecordingFinishResult::Cancelled;
        }
        if !send_if_valid {
            self.cancel_recording(cx);
            return VoiceRecordingFinishResult::Cancelled;
        }
        self.finish_recording(true, cx)
    }

    pub fn pause_or_resume(&mut self, cx: &mut Context<Self>) {
        match self.phase {
            VoiceMessageRecorderPhase::Recording { is_locked } => self.pause_recording(is_locked, cx),
            VoiceMessageRecorderPhase::Paused { is_locked } => self.resume_recording(is_locked, cx),
            VoiceMessageRecorderPhase::Arming | VoiceMessageRecorderPhase::Idle => {}
        }
    }

    pub fn pause_for_background(&mut self, cx: &mut Context<Self>) {
        match self.phase {
            VoiceMessageRecorderPhase::Arming => self.cancel_recording(cx),
            VoiceMessageRecorderPhase::Recording { .. } => self.pause_recording(true, cx),
            VoiceMessageRecorderPhase::Paused { .. } | VoiceMessageRecorderPhase::Idle => {}
        }
    }

    pub fn cancel_recording(&mut self, cx: &mut Context<Self>) {
        self.start_generation = self.start_generation.wrapping_add(1);
        self.stop_timers();
        AudioRecorderRelease::stop_and_release(self.audio_recorder.take());
        self.deactivate_session(cx);
        self.cleanup_file();
        self.reset_state(cx);
        impact(ImpactStyle::Light);
    }

    pub fn finish_recording(&mut self, require_minimum_duration: bool, cx: &mut Context<Self>) -> VoiceRecordingFinishResult {
        self.stop_timers();
        AudioRecorderRelease::stop_and_release(self.audio_recorder.take());
        self.deactivate_session(cx);

        let duration_ms = self.elapsed_milliseconds;
        let peaks = Self::downsample_peaks(&self.waveform_peaks, 128);

        let Some(url) = self.output_url.clone() else {
            self.reset_state(cx);
            return VoiceRecordingFinishResult::Failed;
        };

        if require_minimum_duration && duration_ms < Self::MIN_DURATION_MILLISECONDS {
            self.cleanup_file();
            self.reset_state(cx);
            impact(ImpactStyle::Rigid);
            return VoiceRecordingFinishResult::TooShort;
        }

        let metadata = VoiceMessageMetadata {
            waveform: peaks,
            duration_seconds: (duration_ms / 1000).max(1),
            duration_milliseconds: duration_ms,
        };
        self.reset_state(cx);
        VoiceRecordingFinishResult::Finished { url, metadata }
    }

    fn prepare_audio_session_if_needed(&mut self, cx: &mut Context<Self>) {
        if self.session_prepared {
            return;
        }
        ChatMusicHooks::suspend_for_external_audio(MusicExternalAudioOwner::CHAT_VOICE_RECORDER, true, cx);
        self.session_prepared = true;
        if !self.is_active() {
            self.deactivate_session(cx);
        }
    }

    fn start_recorder(&mut self, locked: bool, generation: i64, cx: &mut Context<Self>) -> Task<bool> {
        self.stop_timers();
        self.cleanup_file();

        let url = temporary_directory().join(format!("voice-{}.m4a", Uuid::new_v4().uuid_string()));
        self.output_url = Some(url.clone());

        let recorder_url = url.clone();
        let prepared = cx.background_executor().spawn(async move {
            match AudioRecorder::start(recorder_url, AacSettings::voice_message()) {
                Ok(recorder) => Some(recorder),
                Err(error) => {
                    tracing::warn!(target: "riseonly::X1849", %error, "the voice recorder did not start");
                    None
                }
            }
        });
        cx.spawn(async move |this, cx| {
            let prepared = prepared.await;
            match this.update(cx, |recorder, cx| recorder.finish_start_recorder(prepared, &url, locked, generation, cx)) {
                Ok(started) => started,
                Err(_) => {
                    FileManager.remove_item(&url).ok();
                    false
                }
            }
        })
    }

    fn finish_start_recorder(
        &mut self,
        prepared: Option<AudioRecorder>,
        url: &Path,
        locked: bool,
        generation: i64,
        cx: &mut Context<Self>,
    ) -> bool {
        if generation != self.start_generation {
            AudioRecorderRelease::stop_and_release(prepared);
            FileManager.remove_item(url).ok();
            return false;
        }

        let Some(recorder) = prepared else {
            self.deactivate_session(cx);
            self.cleanup_file();
            self.reset_state(cx);
            return false;
        };

        if self.phase != VoiceMessageRecorderPhase::Arming {
            AudioRecorderRelease::stop_and_release(Some(recorder));
            self.deactivate_session(cx);
            self.cleanup_file();
            return false;
        }

        self.audio_recorder = Some(recorder);
        self.recording_started_at = Some(RiseClock::monotonic());
        self.accumulated_milliseconds = 0;
        self.elapsed_milliseconds = 0;
        self.waveform_peaks.clear();
        self.phase = VoiceMessageRecorderPhase::Recording { is_locked: locked };
        self.start_timers(cx);
        impact(ImpactStyle::Soft);
        cx.notify();
        true
    }

    fn pause_recording(&mut self, locked: bool, cx: &mut Context<Self>) {
        let Some(recorder) = &self.audio_recorder else {
            return;
        };
        if recorder.is_paused() {
            return;
        }
        recorder.pause();
        if let Some(started) = self.recording_started_at {
            self.accumulated_milliseconds += Self::milliseconds_since(started);
        }
        self.recording_started_at = None;
        self.stop_timers();
        self.phase = VoiceMessageRecorderPhase::Paused { is_locked: locked };
        cx.notify();
    }

    fn resume_recording(&mut self, locked: bool, cx: &mut Context<Self>) {
        let Some(recorder) = &self.audio_recorder else {
            return;
        };
        if let Err(error) = recorder.resume() {
            tracing::debug!(%error, "the voice recorder did not resume");
            return;
        }
        self.recording_started_at = Some(RiseClock::monotonic());
        self.phase = VoiceMessageRecorderPhase::Recording { is_locked: locked };
        self.start_timers(cx);
        cx.notify();
    }

    fn start_timers(&mut self, cx: &mut Context<Self>) {
        self.stop_timers();
        self.meter_timer = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(METER_INTERVAL).await;
                if this.update(cx, |recorder, cx| recorder.tick_meter(cx)).is_err() {
                    break;
                }
            }
        }));
        self.pulse_timer = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(PULSE_INTERVAL).await;
                let ticked = this.update(cx, |recorder, _| {
                    recorder.pulse_phase = (recorder.pulse_phase + 0.035) % 1.0;
                });
                if ticked.is_err() {
                    break;
                }
            }
        }));
    }

    fn stop_timers(&mut self) {
        self.meter_timer = None;
        self.pulse_timer = None;
    }

    fn tick_meter(&mut self, cx: &mut Context<Self>) {
        let Some(recorder) = &self.audio_recorder else {
            return;
        };
        let power = recorder.average_power_db();
        self.waveform_peaks.push(Self::normalized_power(power));

        let mut total = self.accumulated_milliseconds;
        if let Some(started) = self.recording_started_at {
            total += Self::milliseconds_since(started);
        }
        self.elapsed_milliseconds = total;

        if total >= Self::MAX_DURATION_MILLISECONDS && matches!(self.phase, VoiceMessageRecorderPhase::Recording { .. }) {
            self.pause_recording(true, cx);
            self.drag_translation = Size::default();
            self.shows_cancel_hint = false;
            self.shows_lock_hint = false;
            impact(ImpactStyle::Rigid);
            cx.notify();
        }
    }

    fn normalized_power(power: f32) -> f32 {
        let min_db: f32 = -55.0;
        let clamped = min_db.max(power);
        ((clamped - min_db) / -min_db).max(0.0).min(1.0)
    }

    fn downsample_peaks(peaks: &[f32], max_count: usize) -> Vec<f32> {
        if peaks.is_empty() {
            return Vec::new();
        }
        if peaks.len() <= max_count {
            return peaks.to_vec();
        }
        let step = peaks.len() as f64 / max_count as f64;
        (0..max_count)
            .map(|index| {
                let start = (index as f64 * step) as usize;
                let end = peaks.len().min(((index + 1) as f64 * step) as usize);
                if end <= start {
                    return peaks[start.min(peaks.len() - 1)];
                }
                let slice = &peaks[start..end];
                slice.iter().sum::<f32>() / slice.len() as f32
            })
            .collect()
    }

    fn deactivate_session(&mut self, cx: &mut Context<Self>) {
        if !self.session_prepared && !self.is_active() {
            return;
        }
        self.session_prepared = false;
        if RiseCallAudioOwnership::is_call_active() {
            return;
        }
        cx.defer(|cx| {
            ChatMusicHooks::resume_after_external_audio(MusicExternalAudioOwner::CHAT_VOICE_RECORDER, cx);
        });
    }

    fn cleanup_file(&mut self) {
        if let Some(url) = &self.output_url {
            FileManager.remove_item(url).ok();
        }
        self.output_url = None;
    }

    fn reset_state(&mut self, cx: &mut Context<Self>) {
        self.phase = VoiceMessageRecorderPhase::Idle;
        self.elapsed_milliseconds = 0;
        self.accumulated_milliseconds = 0;
        self.waveform_peaks.clear();
        self.drag_translation = Size::default();
        self.shows_cancel_hint = false;
        self.shows_lock_hint = false;
        self.recording_started_at = None;
        self.output_url = None;
        self.pulse_phase = 0.0;
        cx.notify();
    }

    fn milliseconds_since(started: Duration) -> i64 {
        i64::try_from(RiseClock::monotonic().saturating_sub(started).as_millis()).unwrap_or(i64::MAX)
    }
}

/// recorder input callback has a weak slot, the queue can still deliver after stop
/// so we hold the last ref until callbacks drain
/// stop is sync since the file must be finalized before the caller reads it
pub enum AudioRecorderRelease {}

impl AudioRecorderRelease {
    pub fn stop_and_release(recorder: Option<AudioRecorder>) {
        let Some(recorder) = recorder else {
            return;
        };
        if let Err(error) = recorder.stop() {
            tracing::debug!(%error, "the voice take did not close cleanly");
        }
    }
}

#[derive(Clone, PartialEq, Debug)]
pub enum VoiceRecordingFinishResult {
    Ignored,
    Cancelled,
    TooShort,
    Failed,
    Finished { url: PathBuf, metadata: VoiceMessageMetadata },
}

pub enum VoiceRecordingTimeFormat {}

impl VoiceRecordingTimeFormat {
    pub fn centiseconds_label(milliseconds: i64) -> String {
        let cs = (milliseconds / 10) % 100;
        let total_seconds = milliseconds / 1000;
        let seconds = total_seconds % 60;
        let minutes = total_seconds / 60;
        format!("{minutes}:{seconds:02},{cs:02}")
    }
}
