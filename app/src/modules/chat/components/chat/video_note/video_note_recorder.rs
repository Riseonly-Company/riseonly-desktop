use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use futures::FutureExt;
use futures::channel::oneshot;
use futures::future::Shared;
use gpui::{App, AsyncApp, Context, Entity, Pixels, Size, Task, WeakEntity};
use rise_core::RiseIsolated;
use rise_core::foundation::foundation_date::RiseClock;
use rise_limits::limit::CHAT_VIDEO_NOTE_DURATION;
use rise_media::audio_capture::{AacSettings, AudioRecorder};
use rise_media::camera_capture::{
    CameraCaptureError, CameraFrame, FrameSink, RecordedSegment, VideoNoteCamera, segment_extension,
};
use rise_platform::HostOs;
use rise_platform::capture_permission::{self, CaptureAccess, CaptureDecision, CaptureKind};
use rise_platform::file_manager::FileManager;
use rise_platform::haptics::{ImpactStyle, impact};
use rise_platform::paths::temporary_directory;

use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::core::lib::sai::sai::Sai;
use crate::modules::chat::components::chat::voice::voice_message_recorder::VoiceMessageRecorder;
use crate::modules::chat::shared::chat_music_hooks::ChatMusicHooks;
use crate::modules::media_picker::export::video_note_composition::{
    VideoNoteCaptureRecipe, VideoNoteCaptureSegment, VideoNoteComposition,
};
use crate::modules::music::shared::utils::music::music_external_audio_owner::MusicExternalAudioOwner;

const LOG_NAME: &str = "VideoNote";
const METER_INTERVAL: Duration = Duration::from_millis(50);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum VideoNoteRecorderPhase {
    Idle,
    Arming,
    Recording { is_locked: bool },
    Paused { is_locked: bool },
    Finishing,
}

#[derive(Clone, PartialEq)]
pub enum VideoNoteFinishResult {
    Ignored,
    Cancelled,
    TooShort,
    Failed,
    Finishing,
    Finished {
        recipe: VideoNoteCaptureRecipe,
        poster_url: Option<PathBuf>,
        duration_seconds: f64,
    },
}

#[derive(Default)]
struct VideoNotePreviewSlot {
    frame: Option<CameraFrame>,
    generation: u64,
}

/// what the recording overlay shows, the latest camera frame
#[derive(Clone, Default)]
pub struct VideoNoteCaptureSession {
    preview: Arc<Mutex<VideoNotePreviewSlot>>,
}

impl VideoNoteCaptureSession {
    pub fn latest_frame(&self) -> Option<CameraFrame> {
        self.preview.lock().unwrap_or_else(PoisonError::into_inner).frame.clone()
    }

    /// changes with every delivered frame, the overlay redraws when it moves
    pub fn frame_generation(&self) -> u64 {
        self.preview.lock().unwrap_or_else(PoisonError::into_inner).generation
    }

    pub fn is_running(&self) -> bool {
        self.preview.lock().unwrap_or_else(PoisonError::into_inner).frame.is_some()
    }

    pub(crate) fn frame_sink(&self) -> FrameSink {
        let preview = Arc::clone(&self.preview);
        Arc::new(move |frame: CameraFrame| {
            let mut slot = preview.lock().unwrap_or_else(PoisonError::into_inner);
            slot.frame = Some(frame);
            slot.generation = slot.generation.wrapping_add(1);
        })
    }

    pub(crate) fn clear(&self) {
        self.preview.lock().unwrap_or_else(PoisonError::into_inner).frame = None;
    }
}

#[derive(Default)]
struct VideoNoteCaptureHardware {
    camera: Option<VideoNoteCamera>,
    audio_recorder: Option<AudioRecorder>,
    segment_start_offsets: Vec<f64>,
}

/// serial capture queue, every job opens devices or touches the disk
/// so it runs on the background executor one after another in order
#[derive(Default)]
struct VideoNoteCaptureQueue {
    hardware: Arc<RiseIsolated<VideoNoteCaptureHardware>>,
    tail: Option<Shared<oneshot::Receiver<()>>>,
}

impl VideoNoteCaptureQueue {
    fn run<R: Send + 'static>(
        &mut self,
        cx: &App,
        body: impl FnOnce(&mut VideoNoteCaptureHardware) -> R + Send + 'static,
    ) -> Task<R> {
        let previous = self.tail.take();
        let (done, finished) = oneshot::channel::<()>();
        self.tail = Some(finished.shared());
        let hardware = Arc::clone(&self.hardware);
        cx.background_executor().spawn(async move {
            if let Some(previous) = previous {
                let _ = previous.await;
            }
            let result = hardware.with(body);
            let _ = done.send(());
            result
        })
    }
}

fn remove_files(paths: &[PathBuf]) {
    for path in paths {
        let _ = FileManager.remove_item(path);
    }
}

fn raw_segment_location() -> (PathBuf, String) {
    let template = VideoNoteComposition::temporary_url("raw", segment_extension(HostOs::current()));
    let directory = template.parent().map(Path::to_path_buf).unwrap_or_else(temporary_directory);
    let stem = template
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_default();
    (directory, stem)
}

fn recipe_segments(segments: &[RecordedSegment], offsets: &[f64]) -> Vec<VideoNoteCaptureSegment> {
    segments
        .iter()
        .enumerate()
        .map(|(index, segment)| VideoNoteCaptureSegment {
            url: segment.path.clone(),
            is_front_camera: segment.front,
            start_offset: offsets.get(index).copied().unwrap_or(0.0),
        })
        .collect()
}

pub struct VideoNoteRecorder {
    phase: VideoNoteRecorderPhase,
    elapsed_milliseconds: i64,
    drag_translation: Size<Pixels>,
    shows_cancel_hint: bool,
    shows_lock_hint: bool,
    uses_front_camera: bool,
    torch_enabled: bool,
    can_flip_camera: bool,

    capture_session: VideoNoteCaptureSession,
    capture_queue: VideoNoteCaptureQueue,
    recording_started_at: Option<Duration>,
    accumulated_milliseconds: i64,
    meter_timer: Option<Task<()>>,
    audio_url: Option<PathBuf>,
    is_starting: bool,
    start_generation: u64,
    camera_access_was_denied: bool,
    export_duration_ms: i64,

    is_flipping: bool,
    flip_preserved_ms: i64,
}

impl VideoNoteRecorder {
    pub const MIN_DURATION_MILLISECONDS: i64 = 1000;
    pub const MAX_DURATION_MILLISECONDS: i64 = (CHAT_VIDEO_NOTE_DURATION / 1000) as i64 * 1000;
    pub const LOCK_SWIPE_THRESHOLD: f32 = VoiceMessageRecorder::LOCK_SWIPE_THRESHOLD;
    pub const CANCEL_SWIPE_THRESHOLD: f32 = VoiceMessageRecorder::CANCEL_SWIPE_THRESHOLD;

    pub fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            phase: VideoNoteRecorderPhase::Idle,
            elapsed_milliseconds: 0,
            drag_translation: Size::default(),
            shows_cancel_hint: false,
            shows_lock_hint: false,
            uses_front_camera: true,
            torch_enabled: false,
            can_flip_camera: false,
            capture_session: VideoNoteCaptureSession::default(),
            capture_queue: VideoNoteCaptureQueue::default(),
            recording_started_at: None,
            accumulated_milliseconds: 0,
            meter_timer: None,
            audio_url: None,
            is_starting: false,
            start_generation: 0,
            camera_access_was_denied: false,
            export_duration_ms: 0,
            is_flipping: false,
            flip_preserved_ms: 0,
        }
    }

    pub fn phase(&self) -> VideoNoteRecorderPhase {
        self.phase
    }

    pub fn elapsed_milliseconds(&self) -> i64 {
        self.elapsed_milliseconds
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

    pub fn uses_front_camera(&self) -> bool {
        self.uses_front_camera
    }

    pub fn torch_enabled(&self) -> bool {
        self.torch_enabled
    }

    pub fn capture_session(&self) -> &VideoNoteCaptureSession {
        &self.capture_session
    }

    pub fn camera_access_was_denied(&self) -> bool {
        self.camera_access_was_denied
    }

    /// true when a second camera exists to flip to, the flip button is hidden without one
    pub fn can_flip_camera(&self) -> bool {
        self.can_flip_camera
    }

    pub fn supports_torch(&self) -> bool {
        false
    }

    /// what the screen shows, finishing is excluded on purpose
    /// shield, blur and capture mode composer must leave the same frame the button was released
    pub fn is_active(&self) -> bool {
        match self.phase {
            VideoNoteRecorderPhase::Idle | VideoNoteRecorderPhase::Finishing => false,
            VideoNoteRecorderPhase::Arming
            | VideoNoteRecorderPhase::Recording { .. }
            | VideoNoteRecorderPhase::Paused { .. } => true,
        }
    }

    /// recorder still owns the camera and the file, a new recording must not start
    pub fn is_busy(&self) -> bool {
        match self.phase {
            VideoNoteRecorderPhase::Idle => false,
            VideoNoteRecorderPhase::Arming
            | VideoNoteRecorderPhase::Recording { .. }
            | VideoNoteRecorderPhase::Paused { .. }
            | VideoNoteRecorderPhase::Finishing => true,
        }
    }

    pub fn is_locked(&self) -> bool {
        match self.phase {
            VideoNoteRecorderPhase::Recording { is_locked } | VideoNoteRecorderPhase::Paused { is_locked } => {
                is_locked
            }
            VideoNoteRecorderPhase::Arming | VideoNoteRecorderPhase::Idle | VideoNoteRecorderPhase::Finishing => {
                false
            }
        }
    }

    pub fn is_locked_or_finishing(&self) -> bool {
        match self.phase {
            VideoNoteRecorderPhase::Recording { is_locked } | VideoNoteRecorderPhase::Paused { is_locked } => {
                is_locked
            }
            VideoNoteRecorderPhase::Finishing => true,
            VideoNoteRecorderPhase::Arming | VideoNoteRecorderPhase::Idle => false,
        }
    }

    pub fn request_camera_permission_if_needed(&mut self, cx: &mut Context<Self>) -> Task<bool> {
        cx.spawn(async move |this, cx| {
            let decision = capture_permission::ensure_video_note_access().await;
            let _ = this.update(cx, |recorder, _| {
                recorder.camera_access_was_denied = matches!(decision, Ok(CaptureDecision::Blocked));
            });
            matches!(decision, Ok(CaptureDecision::Granted))
        })
    }

    pub fn begin_hold_recording(&mut self, cx: &mut Context<Self>) -> Task<bool> {
        match self.phase {
            VideoNoteRecorderPhase::Arming
            | VideoNoteRecorderPhase::Recording { .. }
            | VideoNoteRecorderPhase::Paused { .. }
            | VideoNoteRecorderPhase::Finishing => return Task::ready(true),
            VideoNoteRecorderPhase::Idle => {}
        }
        if self.is_starting {
            return Task::ready(true);
        }
        self.is_starting = true;
        self.phase = VideoNoteRecorderPhase::Arming;
        let generation = self.start_generation;
        cx.notify();

        let (answer, answered) = oneshot::channel::<bool>();
        cx.spawn(async move |this, cx| {
            let started = Self::run_hold_start(this.clone(), generation, cx).await;
            let _ = this.update(cx, |recorder, _| recorder.is_starting = false);
            let _ = answer.send(started);
        })
        .detach();
        cx.spawn(async move |_, _| answered.await.unwrap_or(false))
    }

    async fn run_hold_start(this: WeakEntity<Self>, generation: u64, cx: &mut AsyncApp) -> bool {
        let Ok(permission) = this.update(cx, |recorder, cx| recorder.request_camera_permission_if_needed(cx)) else {
            return false;
        };
        let granted = permission.await;
        let configure = this.update(cx, |recorder, cx| {
            if !granted {
                Sai::log(LOG_NAME, || "beginHoldRecording: camera/mic permission DENIED", SaiLogKind::Ui, cx);
                recorder.reset_state(cx);
                return None;
            }
            if recorder.start_generation != generation {
                recorder.reset_state(cx);
                return None;
            }
            recorder.cleanup_files(cx);
            Some(recorder.ensure_selected_camera_input(cx))
        });
        let Ok(Some(configure)) = configure else {
            return false;
        };
        configure.await;

        let start = this.update(cx, |recorder, cx| {
            if recorder.start_generation != generation {
                recorder.reset_state(cx);
                return None;
            }
            Some(recorder.start_recording(false, cx))
        });
        let Ok(Some(start)) = start else {
            return false;
        };
        let started = start.await;
        this.update(cx, |recorder, cx| {
            if recorder.start_generation != generation {
                if started {
                    recorder.cancel_recording(cx);
                }
                return false;
            }
            Sai::log(
                LOG_NAME,
                || format!("beginHoldRecording: startRecording returned {started}"),
                SaiLogKind::Ui,
                cx,
            );
            started
        })
        .unwrap_or(false)
    }

    pub fn lock_recording(&mut self, cx: &mut Context<Self>) {
        let VideoNoteRecorderPhase::Recording { .. } = self.phase else {
            return;
        };
        self.phase = VideoNoteRecorderPhase::Recording { is_locked: true };
        self.drag_translation = Size::default();
        self.shows_cancel_hint = false;
        self.shows_lock_hint = false;
        let _ = impact(ImpactStyle::Medium);
        cx.notify();
    }

    pub fn update_drag(&mut self, translation: Size<Pixels>, cx: &mut Context<Self>) {
        self.drag_translation = translation;
        let width = f32::from(translation.width);
        let height = f32::from(translation.height);
        self.shows_cancel_hint = width < -Self::CANCEL_SWIPE_THRESHOLD * 0.45;

        if !self.is_locked() {
            self.shows_lock_hint = height < -Self::LOCK_SWIPE_THRESHOLD * 0.45;
            if matches!(self.phase, VideoNoteRecorderPhase::Recording { .. }) && height < -Self::LOCK_SWIPE_THRESHOLD
            {
                self.lock_recording(cx);
            }
        }
        cx.notify();
    }

    pub fn end_hold(&mut self, send_if_valid: bool, cx: &mut Context<Self>) -> VideoNoteFinishResult {
        if self.phase == VideoNoteRecorderPhase::Arming {
            let cancelled_by_swipe = f32::from(self.drag_translation.width) < -Self::CANCEL_SWIPE_THRESHOLD;
            let access_granted = capture_permission::status(CaptureKind::Camera) == Ok(CaptureAccess::Granted);
            self.cancel_recording(cx);
            if !send_if_valid || !access_granted || cancelled_by_swipe {
                return VideoNoteFinishResult::Cancelled;
            }
            let _ = impact(ImpactStyle::Rigid);
            return VideoNoteFinishResult::TooShort;
        }
        let VideoNoteRecorderPhase::Recording { is_locked } = self.phase else {
            return VideoNoteFinishResult::Ignored;
        };
        if is_locked {
            return VideoNoteFinishResult::Ignored;
        }
        if f32::from(self.drag_translation.width) < -Self::CANCEL_SWIPE_THRESHOLD {
            self.cancel_recording(cx);
            return VideoNoteFinishResult::Cancelled;
        }
        if !send_if_valid {
            self.cancel_recording(cx);
            return VideoNoteFinishResult::Cancelled;
        }
        self.finish_recording(true, cx)
    }

    pub fn pause_or_resume(&mut self, cx: &mut Context<Self>) {
        match self.phase {
            VideoNoteRecorderPhase::Recording { is_locked } => self.pause_recording(is_locked, cx),
            VideoNoteRecorderPhase::Paused { is_locked } => self.resume_recording(is_locked, cx),
            VideoNoteRecorderPhase::Arming | VideoNoteRecorderPhase::Idle | VideoNoteRecorderPhase::Finishing => {}
        }
    }

    /// a call taking the microphone keeps the take as a locked pause to send or discard
    pub fn pause_for_interruption(&mut self, cx: &mut Context<Self>) {
        match self.phase {
            VideoNoteRecorderPhase::Arming => self.cancel_recording(cx),
            VideoNoteRecorderPhase::Recording { .. } => self.pause_recording(true, cx),
            VideoNoteRecorderPhase::Paused { .. } | VideoNoteRecorderPhase::Idle | VideoNoteRecorderPhase::Finishing => {}
        }
    }

    pub fn pause_for_background(&mut self, cx: &mut Context<Self>) {
        self.pause_for_interruption(cx);
        if !matches!(self.phase, VideoNoteRecorderPhase::Paused { .. }) {
            return;
        }
        self.capture_queue
            .run(cx, |hardware| {
                if let Some(camera) = hardware.camera.as_mut()
                    && let Err(error) = camera.suspend(RiseClock::monotonic())
                {
                    tracing::debug!(%error, "the video note segment did not close on suspend");
                }
            })
            .detach();
    }

    pub fn resume_after_background(&mut self, cx: &mut Context<Self>) {
        if !matches!(self.phase, VideoNoteRecorderPhase::Paused { .. }) {
            return;
        }
        let reactivate = self.capture_queue.run(cx, |hardware| match hardware.camera.as_mut() {
            Some(camera) if camera.is_suspended() => camera.reactivate().map(|()| Some(camera.can_flip())),
            _ => Ok(None),
        });
        cx.spawn(async move |this, cx| {
            let reactivated = reactivate.await;
            let _ = this.update(cx, |recorder, cx| match reactivated {
                Ok(Some(can_flip)) => {
                    recorder.can_flip_camera = can_flip;
                    recorder.apply_torch(cx);
                    cx.notify();
                }
                Ok(None) => {}
                Err(error) => Sai::log(
                    LOG_NAME,
                    || format!("resumeAfterBackground: camera did not reopen: {error}"),
                    SaiLogKind::Ui,
                    cx,
                ),
            });
        })
        .detach();
    }

    pub fn flip_camera(&mut self, cx: &mut Context<Self>) {
        if self.is_flipping {
            return;
        }
        match self.phase {
            VideoNoteRecorderPhase::Recording { is_locked } => {
                self.is_flipping = true;
                self.flip_preserved_ms = self.elapsed_milliseconds;
                self.uses_front_camera = !self.uses_front_camera;
                self.torch_enabled = false;
                self.stop_timer();
                cx.notify();
                let flip = self.capture_queue.run(cx, |hardware| {
                    let camera = hardware.camera.as_mut().ok_or(CameraCaptureError::NoCamera)?;
                    camera.flip(RiseClock::monotonic())?;
                    let offset = hardware
                        .audio_recorder
                        .as_ref()
                        .map_or(0.0, |recorder| recorder.elapsed().as_secs_f64());
                    hardware.segment_start_offsets.push(offset);
                    Ok::<bool, CameraCaptureError>(camera.can_flip())
                });
                cx.spawn(async move |this, cx| {
                    let flipped = flip.await;
                    let _ = this.update(cx, |recorder, cx| recorder.handle_flip_finished(flipped, is_locked, cx));
                })
                .detach();
            }
            VideoNoteRecorderPhase::Paused { .. } => {
                self.uses_front_camera = !self.uses_front_camera;
                self.torch_enabled = false;
                cx.notify();
                self.reconfigure_camera_input(cx).detach();
            }
            VideoNoteRecorderPhase::Idle | VideoNoteRecorderPhase::Arming | VideoNoteRecorderPhase::Finishing => {}
        }
    }

    fn handle_flip_finished(
        &mut self,
        flipped: Result<bool, CameraCaptureError>,
        was_locked: bool,
        cx: &mut Context<Self>,
    ) {
        self.is_flipping = false;
        match flipped {
            Ok(can_flip) => {
                self.can_flip_camera = can_flip;
                let preserved_ms = self.flip_preserved_ms;
                self.recording_started_at = Some(RiseClock::monotonic());
                self.accumulated_milliseconds = preserved_ms;
                self.elapsed_milliseconds = preserved_ms;
                self.phase = VideoNoteRecorderPhase::Recording { is_locked: was_locked };
                self.start_timer(cx);
                if !was_locked && f32::from(self.drag_translation.height) < -Self::LOCK_SWIPE_THRESHOLD {
                    self.lock_recording(cx);
                }
                cx.notify();
            }
            Err(error) => {
                Sai::log(LOG_NAME, || format!("flipCamera: camera switch FAILED: {error}"), SaiLogKind::Ui, cx);
                self.cleanup_files(cx);
                self.reset_state(cx);
                VideoNoteMovieDelegate::file_output(false, cx);
            }
        }
    }

    pub fn set_torch_enabled(&mut self, enabled: bool, cx: &mut Context<Self>) {
        if self.uses_front_camera {
            self.torch_enabled = false;
            cx.notify();
            return;
        }
        self.torch_enabled = enabled;
        cx.notify();
        self.apply_torch(cx);
    }

    pub fn cancel_recording(&mut self, cx: &mut Context<Self>) {
        self.stop_timer();
        self.start_generation = self.start_generation.wrapping_add(1);
        self.cleanup_files(cx);
        self.reset_state(cx);
        let _ = impact(ImpactStyle::Light);
    }

    pub fn finish_recording(&mut self, require_minimum_duration: bool, cx: &mut Context<Self>) -> VideoNoteFinishResult {
        match self.phase {
            VideoNoteRecorderPhase::Recording { .. } | VideoNoteRecorderPhase::Paused { .. } => {}
            phase => {
                Sai::log(LOG_NAME, || format!("finishRecording: IGNORED, phase={phase:?}"), SaiLogKind::Ui, cx);
                return VideoNoteFinishResult::Ignored;
            }
        }
        let duration_ms = self.elapsed_milliseconds;
        let phase = self.phase;
        Sai::log(
            LOG_NAME,
            || format!("finishRecording: durationMs={duration_ms} phase={phase:?}"),
            SaiLogKind::Ui,
            cx,
        );
        if require_minimum_duration && duration_ms < Self::MIN_DURATION_MILLISECONDS {
            Sai::log(
                LOG_NAME,
                || {
                    format!(
                        "finishRecording: TOO SHORT ({duration_ms}ms < {}ms)",
                        Self::MIN_DURATION_MILLISECONDS
                    )
                },
                SaiLogKind::Ui,
                cx,
            );
            self.cancel_recording(cx);
            let _ = impact(ImpactStyle::Rigid);
            return VideoNoteFinishResult::TooShort;
        }
        self.export_duration_ms = duration_ms;
        self.stop_timer();

        let stop = self.capture_queue.run(cx, |hardware| match hardware.camera.as_mut() {
            Some(camera) => match camera.pause(RiseClock::monotonic()) {
                Ok(()) | Err(CameraCaptureError::NotRecording) => true,
                Err(error) => {
                    tracing::debug!(%error, "the last video note segment did not close");
                    false
                }
            },
            None => false,
        });
        cx.spawn(async move |_, cx| {
            let success = stop.await;
            cx.update(|cx| VideoNoteMovieDelegate::file_output(success, cx));
        })
        .detach();
        self.phase = VideoNoteRecorderPhase::Finishing;
        cx.notify();
        VideoNoteFinishResult::Finishing
    }

    pub fn handle_movie_finished(&mut self, success: bool, cx: &mut Context<Self>) -> Task<VideoNoteFinishResult> {
        let is_flipping = self.is_flipping;
        Sai::log(
            LOG_NAME,
            || format!("handleMovieFinished: success={success} isFlipping={is_flipping}"),
            SaiLogKind::Ui,
            cx,
        );
        if !success {
            Sai::log(LOG_NAME, || "handleMovieFinished: FAILED at final guard", SaiLogKind::Ui, cx);
            self.teardown_capture(cx).detach();
            self.cleanup_files(cx);
            self.reset_state(cx);
            return Task::ready(VideoNoteFinishResult::Failed);
        }

        let audio_url = self.audio_url.take();
        let collect = self.capture_queue.run(cx, |hardware| {
            let now = RiseClock::monotonic();
            let segments = match hardware.camera.take() {
                Some(camera) => camera.finish(now).unwrap_or_else(|error| {
                    tracing::debug!(%error, "the video note camera did not close");
                    Vec::new()
                }),
                None => Vec::new(),
            };
            let offsets = std::mem::take(&mut hardware.segment_start_offsets);
            let audio_closed = match hardware.audio_recorder.take() {
                Some(recorder) => recorder.stop().is_ok(),
                None => false,
            };
            (segments, offsets, audio_closed)
        });
        let export_duration_ms = self.export_duration_ms;
        cx.spawn(async move |this, cx| {
            let (segments, offsets, audio_closed) = collect.await;
            let Ok(session) = this.update(cx, |recorder, _| recorder.capture_session.clone()) else {
                return VideoNoteFinishResult::Failed;
            };
            session.clear();
            if segments.is_empty() {
                let _ = this.update(cx, |recorder, cx| {
                    Sai::log(LOG_NAME, || "handleMovieFinished: FAILED, no segment was written", SaiLogKind::Ui, cx);
                    recorder.audio_url = audio_url;
                    recorder.cleanup_files(cx);
                    recorder.reset_state(cx);
                });
                return VideoNoteFinishResult::Failed;
            }
            let recipe = VideoNoteCaptureRecipe {
                segments: recipe_segments(&segments, &offsets),
                audio_url: audio_url.filter(|_| audio_closed),
            };
            let duration_sec = (export_duration_ms / 1000).max(1);
            let segment_count = recipe.segments.len();
            let has_audio = recipe.audio_url.is_some();
            let _ = this.update(cx, |recorder, cx| {
                recorder.reset_state(cx);
                Sai::log(
                    LOG_NAME,
                    || {
                        format!(
                            "handleMovieFinished: handing off {segment_count} segment(s), audio={has_audio}, durationSec={duration_sec}"
                        )
                    },
                    SaiLogKind::Ui,
                    cx,
                );
            });
            let poster_url = VideoNoteComposition::poster(&recipe).await;
            VideoNoteFinishResult::Finished {
                recipe,
                poster_url,
                duration_seconds: duration_sec as f64,
            }
        })
    }

    /// camera and microphone, closing either blocks so it runs on the capture queue
    fn teardown_capture(&mut self, cx: &mut Context<Self>) -> Task<()> {
        self.capture_session.clear();
        self.capture_queue.run(cx, |hardware| {
            hardware.camera = None;
            if let Some(recorder) = hardware.audio_recorder.take()
                && let Err(error) = recorder.stop()
            {
                tracing::debug!(%error, "the video note sound did not close");
            }
        })
    }

    fn ensure_selected_camera_input(&mut self, cx: &mut Context<Self>) -> Task<()> {
        self.reconfigure_camera_input(cx)
    }

    /// opens the selected camera or switches the open one to it, the preview starts with it
    fn reconfigure_camera_input(&mut self, cx: &mut Context<Self>) -> Task<()> {
        let front = self.uses_front_camera;
        let frames = self.capture_session.frame_sink();
        let (directory, stem) = raw_segment_location();
        let reconfigure = self.capture_queue.run(cx, move |hardware| {
            let Some(camera) = hardware.camera.as_mut() else {
                let camera = VideoNoteCamera::open(front, directory, stem, frames)?;
                let can_flip = camera.can_flip();
                hardware.camera = Some(camera);
                return Ok(can_flip);
            };
            if camera.is_front() != front {
                camera.flip(RiseClock::monotonic())?;
            }
            Ok::<bool, CameraCaptureError>(camera.can_flip())
        });
        cx.spawn(async move |this, cx| {
            let reconfigured = reconfigure.await;
            let _ = this.update(cx, |recorder, cx| match reconfigured {
                Ok(can_flip) => {
                    recorder.can_flip_camera = can_flip;
                    recorder.apply_torch(cx);
                    cx.notify();
                }
                Err(error) => Sai::log(
                    LOG_NAME,
                    || format!("reconfigureCameraInput: NO camera device: {error}"),
                    SaiLogKind::Ui,
                    cx,
                ),
            });
        })
    }

    fn apply_torch(&mut self, cx: &mut Context<Self>) {
        let enabled = self.torch_enabled;
        let apply = self.capture_queue.run(cx, move |hardware| match hardware.camera.as_mut() {
            Some(camera) => camera.set_torch(enabled),
            None => Ok(()),
        });
        cx.spawn(async move |this, cx| {
            if apply.await.is_err() {
                let _ = this.update(cx, |recorder, cx| {
                    if recorder.torch_enabled {
                        recorder.torch_enabled = false;
                        cx.notify();
                    }
                });
            }
        })
        .detach();
    }

    fn start_recording(&mut self, locked: bool, cx: &mut Context<Self>) -> Task<bool> {
        if self.phase != VideoNoteRecorderPhase::Arming {
            let phase = self.phase;
            Sai::log(
                LOG_NAME,
                || format!("startRecording: phase not arming ({phase:?}), aborting"),
                SaiLogKind::Ui,
                cx,
            );
            return Task::ready(false);
        }
        let audio_url = if self.audio_url.is_none() {
            self.suspend_music(cx);
            Some(VideoNoteComposition::temporary_url("audio", "m4a"))
        } else {
            None
        };
        let start = self.capture_queue.run(cx, move |hardware| {
            let Some(camera) = hardware.camera.as_mut() else {
                return (None, Err(CameraCaptureError::NoCamera));
            };
            if let Some(failure) = camera.failure() {
                return (None, Err(failure));
            }
            let mut audio_started = None;
            if let Some(url) = audio_url
                && hardware.audio_recorder.is_none()
            {
                match AudioRecorder::start(url.clone(), AacSettings::video_note()) {
                    Ok(recorder) => {
                        hardware.audio_recorder = Some(recorder);
                        audio_started = Some(url);
                    }
                    Err(error) => tracing::debug!(%error, "the video note records without sound"),
                }
            }
            let offset = hardware
                .audio_recorder
                .as_ref()
                .map_or(0.0, |recorder| recorder.elapsed().as_secs_f64());
            let recorded = camera.record(RiseClock::monotonic());
            if recorded.is_ok() {
                hardware.segment_start_offsets.push(offset);
            }
            (audio_started, recorded)
        });
        cx.spawn(async move |this, cx| {
            let (audio_started, recorded) = start.await;
            this.update(cx, |recorder, cx| {
                if audio_started.is_some() {
                    recorder.audio_url = audio_started;
                }
                if let Err(error) = recorded {
                    Sai::log(
                        LOG_NAME,
                        || format!("startRecording: no active video connection ({error}), aborting"),
                        SaiLogKind::Ui,
                        cx,
                    );
                    recorder.teardown_capture(cx).detach();
                    recorder.cleanup_files(cx);
                    recorder.reset_state(cx);
                    return false;
                }
                if recorder.phase != VideoNoteRecorderPhase::Arming {
                    recorder.cancel_recording(cx);
                    return false;
                }
                recorder.recording_started_at = Some(RiseClock::monotonic());
                recorder.accumulated_milliseconds = 0;
                recorder.elapsed_milliseconds = 0;
                recorder.phase = VideoNoteRecorderPhase::Recording { is_locked: locked };
                recorder.start_timer(cx);
                Sai::log(LOG_NAME, || "startRecording: STARTED recording", SaiLogKind::Ui, cx);
                let _ = impact(ImpactStyle::Soft);
                if !locked && f32::from(recorder.drag_translation.height) < -Self::LOCK_SWIPE_THRESHOLD {
                    recorder.lock_recording(cx);
                }
                cx.notify();
                true
            })
            .unwrap_or(false)
        })
    }

    fn pause_recording(&mut self, locked: bool, cx: &mut Context<Self>) {
        if !matches!(self.phase, VideoNoteRecorderPhase::Recording { .. }) {
            return;
        }
        self.capture_queue
            .run(cx, |hardware| {
                if let Some(recorder) = hardware.audio_recorder.as_ref() {
                    recorder.pause();
                }
                if let Some(camera) = hardware.camera.as_mut()
                    && let Err(error) = camera.pause(RiseClock::monotonic())
                {
                    tracing::debug!(%error, "the video note segment did not close on pause");
                }
            })
            .detach();
        if let Some(started) = self.recording_started_at {
            self.accumulated_milliseconds += Self::milliseconds_since(started);
        }
        self.recording_started_at = None;
        self.stop_timer();
        self.phase = VideoNoteRecorderPhase::Paused { is_locked: locked };
        cx.notify();
    }

    fn resume_recording(&mut self, locked: bool, cx: &mut Context<Self>) {
        let resume = self.capture_queue.run(cx, |hardware| {
            if let Some(recorder) = hardware.audio_recorder.as_ref()
                && let Err(error) = recorder.resume()
            {
                tracing::debug!(%error, "the video note sound did not resume");
            }
            let offset = hardware
                .audio_recorder
                .as_ref()
                .map_or(0.0, |recorder| recorder.elapsed().as_secs_f64());
            let camera = hardware.camera.as_mut().ok_or(CameraCaptureError::NoCamera)?;
            camera.record(RiseClock::monotonic())?;
            hardware.segment_start_offsets.push(offset);
            Ok::<(), CameraCaptureError>(())
        });
        self.recording_started_at = Some(RiseClock::monotonic());
        self.phase = VideoNoteRecorderPhase::Recording { is_locked: locked };
        self.start_timer(cx);
        cx.notify();
        cx.spawn(async move |this, cx| {
            if let Err(error) = resume.await {
                let _ = this.update(cx, |recorder, cx| {
                    Sai::log(LOG_NAME, || format!("resumeRecording: FAILED: {error}"), SaiLogKind::Ui, cx);
                    recorder.cleanup_files(cx);
                    recorder.reset_state(cx);
                });
            }
        })
        .detach();
    }

    fn suspend_music(&self, cx: &mut App) {
        ChatMusicHooks::suspend_for_external_audio(MusicExternalAudioOwner::CHAT_VIDEO_NOTE_RECORDER, true, cx);
    }

    fn resume_music(&self, cx: &mut App) {
        ChatMusicHooks::resume_after_external_audio(MusicExternalAudioOwner::CHAT_VIDEO_NOTE_RECORDER, cx);
    }

    fn milliseconds_since(started: Duration) -> i64 {
        i64::try_from(RiseClock::monotonic().saturating_sub(started).as_millis()).unwrap_or(i64::MAX)
    }

    fn start_timer(&mut self, cx: &mut Context<Self>) {
        self.stop_timer();
        self.meter_timer = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(METER_INTERVAL).await;
                if this.update(cx, |recorder, cx| recorder.tick(cx)).is_err() {
                    break;
                }
            }
        }));
    }

    fn stop_timer(&mut self) {
        self.meter_timer = None;
    }

    fn tick(&mut self, cx: &mut Context<Self>) {
        let mut total = self.accumulated_milliseconds;
        if let Some(started) = self.recording_started_at {
            total += Self::milliseconds_since(started);
        }
        self.elapsed_milliseconds = total;
        cx.notify();
        if total >= Self::MAX_DURATION_MILLISECONDS && matches!(self.phase, VideoNoteRecorderPhase::Recording { .. }) {
            let _ = self.finish_recording(false, cx);
            let _ = impact(ImpactStyle::Rigid);
        }
    }

    fn cleanup_files(&mut self, cx: &mut Context<Self>) {
        let audio_url = self.audio_url.take();
        self.capture_session.clear();
        self.capture_queue
            .run(cx, move |hardware| {
                let mut files = hardware.camera.take().map(|camera| camera.files()).unwrap_or_default();
                hardware.segment_start_offsets.clear();
                if let Some(recorder) = hardware.audio_recorder.take()
                    && let Err(error) = recorder.discard()
                {
                    tracing::debug!(%error, "the video note sound file stayed behind");
                }
                files.extend(audio_url);
                remove_files(&files);
            })
            .detach();
    }

    fn reset_state(&mut self, cx: &mut Context<Self>) {
        self.resume_music(cx);
        self.phase = VideoNoteRecorderPhase::Idle;
        self.elapsed_milliseconds = 0;
        self.accumulated_milliseconds = 0;
        self.drag_translation = Size::default();
        self.shows_cancel_hint = false;
        self.shows_lock_hint = false;
        self.recording_started_at = None;
        self.uses_front_camera = true;
        self.torch_enabled = false;
        cx.notify();
    }
}

pub type VideoNoteMovieFinished = Rc<dyn Fn(bool, &mut App)>;

/// where the chat screen learns that the last segment of a take is closed
pub struct VideoNoteMovieDelegate {
    pub on_finished: Option<VideoNoteMovieFinished>,
}

impl VideoNoteMovieDelegate {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, |_| Self { on_finished: None })
    }

    pub fn file_output(success: bool, cx: &mut App) {
        let on_finished = Self::shared(cx).read(cx).on_finished.clone();
        let has_handler = on_finished.is_some();
        Sai::log(
            LOG_NAME,
            || format!("delegate fileOutput: success={success} hasHandler={has_handler}"),
            SaiLogKind::Ui,
            cx,
        );
        if let Some(on_finished) = on_finished {
            on_finished(success, cx);
        }
    }
}
