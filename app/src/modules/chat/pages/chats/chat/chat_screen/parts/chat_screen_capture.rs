use std::rc::Rc;
use gpui::prelude::*;
use gpui::{App, AppContext, Context, Pixels, Window, px};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_i18n::tr;
use rise_ui::native_confirm_alert::NativeConfirmAlert;
use rise_ui::swiftui::prelude::*;
use crate::modules::chat::components::chat::voice::chat_voice_message_player::ChatVoiceMessagePlayer;
use crate::modules::chat::components::chat::composer::chat_composer_types::resign_chat_keyboard;
use crate::modules::chat::components::chat::composer::chat_draft_store::ChatDraftStore;
use crate::modules::chat::components::chat::video_note::video_note_recorder::VideoNoteRecorder;
use crate::modules::chat::components::chat::voice::chat_composer_capture_mode::ChatComposerCaptureMode;
use crate::modules::chat::components::chat::voice::voice_message_recorder::VoiceMessageRecorder;
use crate::modules::chat::components::chat::voice::chat_voice_recording_ui::ChatRecordingClockReader;
use crate::modules::chat::stores::chats::chat_actions::chat_types::ChatPermKey;
use crate::modules::chat::stores::message::message_interactions::message_interactions::MessageInteractionsStore;
use rise_core::foundation::foundation_url::Url;
use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::lib::sai::sai::Sai;
use crate::modules::chat::components::chat::composer::chat_draft_store::ChatVoiceDraft;
use crate::modules::chat::components::chat::video_note::video_note_recorder::{VideoNoteFinishResult, VideoNoteMovieDelegate, VideoNoteMovieFinished};
use crate::modules::chat::components::chat::voice::voice_message_recorder::VoiceRecordingFinishResult;
use crate::modules::chat::stores::message::message_actions::message_types::{MessageMediaItem, MessageMediaKind};
use crate::modules::media_picker::export::video_note_composition::VideoNoteComposition;
use std::time::Duration;
use crate::modules::chat::components::chat::video_note::video_note_recorder::VideoNoteRecorderPhase;
use crate::modules::chat::components::chat::voice::chat_composer_capture_mode::ChatComposerRecordingUIPhase;
use crate::modules::chat::components::chat::voice::voice_message_recorder::VoiceMessageRecorderPhase;
use std::collections::HashMap;
use rise_errors::ErrorCode;
use crate::core::lib::canon_errors::CanonErrors;
use crate::core::widgets::notifier::notifier_store::{Notifier, NotifierPosition, NotifierSendOptions, NotifierType};
use super::super::chat_screen::*;
use super::super::chat_screen_support::*;

impl ChatScreen {
    pub(in super::super) fn is_capture_paused(&self, cx: &App) -> bool {
        match self.capture_mode {
            ChatComposerCaptureMode::Voice => {
                matches!(self.voice_recorder.read(cx).phase(), VoiceMessageRecorderPhase::Paused { .. })
            }
            ChatComposerCaptureMode::VideoNote => {
                matches!(self.video_note_recorder.read(cx).phase(), VideoNoteRecorderPhase::Paused { .. })
            }
        }
    }

    pub(in super::super) fn active_capture_locked(&self, cx: &App) -> bool {
        match self.capture_mode {
            ChatComposerCaptureMode::Voice => self.voice_recorder.read(cx).is_locked(),
            ChatComposerCaptureMode::VideoNote => self.video_note_recorder.read(cx).is_locked_or_finishing(),
        }
    }

    pub(in super::super) fn active_capture_elapsed_ms(&self, cx: &App) -> i64 {
        match self.capture_mode {
            ChatComposerCaptureMode::Voice => self.voice_recorder.read(cx).elapsed_milliseconds(),
            ChatComposerCaptureMode::VideoNote => self.video_note_recorder.read(cx).elapsed_milliseconds(),
        }
    }

    /// clock of the recording strip reads the take itself between renders
    pub(in super::super) fn active_capture_clock_reader(&self) -> ChatRecordingClockReader {
        match self.capture_mode {
            ChatComposerCaptureMode::Voice => {
                let recorder = self.voice_recorder.downgrade();
                Rc::new(move |cx: &App| {
                    let recorder = recorder.upgrade()?;
                    let recorder = recorder.read(cx);
                    recorder.is_active().then(|| recorder.elapsed_milliseconds())
                })
            }
            ChatComposerCaptureMode::VideoNote => {
                let recorder = self.video_note_recorder.downgrade();
                Rc::new(move |cx: &App| {
                    let recorder = recorder.upgrade()?;
                    let recorder = recorder.read(cx);
                    recorder.is_active().then(|| recorder.elapsed_milliseconds())
                })
            }
        }
    }

    pub(in super::super) fn active_capture_drag_translation(&self, cx: &App) -> gpui::Size<Pixels> {
        match self.capture_mode {
            ChatComposerCaptureMode::Voice => self.voice_recorder.read(cx).drag_translation(),
            ChatComposerCaptureMode::VideoNote => self.video_note_recorder.read(cx).drag_translation(),
        }
    }

    pub(in super::super) fn active_capture_shows_cancel_hint(&self, cx: &App) -> bool {
        match self.capture_mode {
            ChatComposerCaptureMode::Voice => self.voice_recorder.read(cx).shows_cancel_hint(),
            ChatComposerCaptureMode::VideoNote => self.video_note_recorder.read(cx).shows_cancel_hint(),
        }
    }

    pub(in super::super) fn is_any_capture_active(&self, cx: &App) -> bool {
        self.voice_recorder.read(cx).is_active() || self.video_note_recorder.read(cx).is_active()
    }

    pub(in super::super) fn recording_ui_phase(&self, cx: &App) -> ChatComposerRecordingUIPhase {
        if self.shows_locked_capture_controls(cx) {
            ChatComposerRecordingUIPhase::Locked
        } else if self.is_any_capture_active(cx) {
            ChatComposerRecordingUIPhase::Holding
        } else {
            ChatComposerRecordingUIPhase::Idle
        }
    }

    /// a take paused at the length limit under the finger shows its send and delete panel at once
    pub(in super::super) fn shows_locked_capture_controls(&self, cx: &App) -> bool {
        self.active_capture_locked(cx) && (!self.capture_finger_down || self.is_capture_paused(cx))
    }

    pub(in super::super) fn is_capture_recording(&self, cx: &App) -> bool {
        match self.capture_mode {
            ChatComposerCaptureMode::Voice => matches!(
                self.voice_recorder.read(cx).phase(),
                VoiceMessageRecorderPhase::Arming | VoiceMessageRecorderPhase::Recording { .. }
            ),
            ChatComposerCaptureMode::VideoNote => matches!(
                self.video_note_recorder.read(cx).phase(),
                VideoNoteRecorderPhase::Arming | VideoNoteRecorderPhase::Recording { .. }
            ),
        }
    }

    pub(in super::super) fn handle_capture_press_began(&mut self, cx: &mut Context<Self>) {
        self.capture_finger_down = true;
        cx.notify();
    }

    pub(in super::super) fn handle_capture_press_ended(&mut self, cx: &mut Context<Self>) {
        self.capture_finger_down = false;
        cx.notify();
    }

    pub(in super::super) fn install_video_note_movie_delegate(&self, window: &Window, cx: &mut Context<Self>) {
        let this = cx.weak_entity();
        let handle = window.window_handle();
        let on_finished: VideoNoteMovieFinished = Rc::new(move |success: bool, cx: &mut App| {
            let Some(screen) = this.upgrade() else {
                return;
            };
            let recorder = screen.read(cx).video_note_recorder.clone();
            let finished = recorder.update(cx, |recorder, cx| recorder.handle_movie_finished(success, cx));
            let this = this.clone();
            cx.spawn(async move |cx| {
                let result = finished.await;
                let _ = handle.update(cx, |_, window, cx| {
                    let _ = this.update(cx, |screen, cx| screen.deliver_video_note_result(result, window, cx));
                });
            })
            .detach();
        });
        VideoNoteMovieDelegate::shared(cx).update(cx, |delegate, _| delegate.on_finished = Some(on_finished));
    }

    pub(in super::super) fn handle_capture_hold_began(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_any_capture_active(cx) || self.video_note_recorder.read(cx).is_busy() {
            return;
        }
        let capture_key = match self.capture_mode {
            ChatComposerCaptureMode::Voice => ChatPermKey::VOICE,
            ChatComposerCaptureMode::VideoNote => ChatPermKey::VIDEO_MESSAGES,
        };
        let chat_id = self.resolved_chat_id(cx);
        let post_topic_id = self.post_topic_id(cx);
        let switched_off = chat_interactions(cx, |store, cx| {
            store.picker_send_switched_off(&chat_id, capture_key, post_topic_id.as_deref(), cx)
        });
        if switched_off {
            self.capture_refused = true;
            Self::show_capture_refused(cx);
            self.capture_finger_down = false;
            cx.notify();
            return;
        }
        resign_chat_keyboard(window, cx);

        match self.capture_mode {
            ChatComposerCaptureMode::Voice => {
                let starting = self.voice_recorder.update(cx, |recorder, cx| recorder.begin_hold_recording(cx));
                self.voice_hold_start_task = Some(cx.spawn(async move |this, cx| {
                    let started = starting.await;
                    let _ = this.update(cx, |screen, cx| {
                        screen.voice_hold_start_task = None;
                        if !started {
                            screen.voice_recorder.update(cx, |recorder, cx| recorder.cancel_recording(cx));
                            screen.capture_finger_down = false;
                            if screen.voice_recorder.read(cx).access_was_denied() {
                                NativeConfirmAlert::present_open_settings(
                                    &tr("chat_mic_access_denied_title"),
                                    &tr("chat_mic_access_denied_message"),
                                    cx,
                                );
                            }
                        }
                        cx.notify();
                    });
                }));
            }
            ChatComposerCaptureMode::VideoNote => {
                self.install_video_note_movie_delegate(window, cx);
                let starting = self.video_note_recorder.update(cx, |recorder, cx| recorder.begin_hold_recording(cx));
                self.video_hold_start_task = Some(cx.spawn(async move |this, cx| {
                    let started = starting.await;
                    let _ = this.update(cx, |screen, cx| {
                        screen.video_hold_start_task = None;
                        if !started {
                            screen.video_note_recorder.update(cx, |recorder, cx| recorder.cancel_recording(cx));
                            screen.capture_finger_down = false;
                            if screen.video_note_recorder.read(cx).camera_access_was_denied() {
                                NativeConfirmAlert::present_open_settings(
                                    &tr("chat_camera_access_denied_title"),
                                    &tr("chat_camera_access_denied_message"),
                                    cx,
                                );
                            }
                        }
                        cx.notify();
                    });
                }));
            }
        }
    }

    pub(in super::super) fn handle_capture_hold_drag(&mut self, translation: gpui::Size<Pixels>, cx: &mut Context<Self>) {
        match self.capture_mode {
            ChatComposerCaptureMode::Voice => self.voice_recorder.update(cx, |recorder, cx| recorder.update_drag(translation, cx)),
            ChatComposerCaptureMode::VideoNote => {
                self.video_note_recorder.update(cx, |recorder, cx| recorder.update_drag(translation, cx));
            }
        }
    }

    pub(in super::super) fn handle_capture_hold_ended(
        &mut self,
        translation: gpui::Size<Pixels>,
        cancelled: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if cancelled {
            self.handle_capture_hold_cancelled(cx);
            return;
        }
        if std::mem::take(&mut self.capture_refused) {
            Self::show_capture_refused(cx);
            return;
        }
        self.video_hold_start_task = None;
        self.handle_capture_hold_drag(translation, cx);
        let was_locked = self.active_capture_locked(cx);
        self.capture_finger_down = false;
        cx.notify();

        match self.capture_mode {
            ChatComposerCaptureMode::Voice => {
                if self.voice_hold_start_task.take().is_some() {
                    self.voice_recorder.update(cx, |recorder, cx| recorder.cancel_recording(cx));
                    self.deliver_voice_recording_result(VoiceRecordingFinishResult::TooShort, window, cx);
                    return;
                }
                let result = self.voice_recorder.update(cx, |recorder, cx| recorder.end_hold(true, cx));
                if !was_locked {
                    self.deliver_voice_recording_result(result, window, cx);
                }
            }
            ChatComposerCaptureMode::VideoNote => {
                let result = self.video_note_recorder.update(cx, |recorder, cx| recorder.end_hold(true, cx));
                if !was_locked && !matches!(result, VideoNoteFinishResult::Finishing) {
                    self.deliver_video_note_result(result, window, cx);
                }
            }
        }
    }

    /// system took the pointer so a take in progress becomes a locked pause
    /// only a swipe past the cancel line deletes
    /// also shows the send restriction banner
    fn show_capture_refused(cx: &mut App) {
        let text = CanonErrors::text(Some(ErrorCode::ChatSendRestricted.wire()), &HashMap::new())
            .unwrap_or_else(|| tr("chat_error_send_restricted"));
        Notifier::send(
            text,
            NotifierSendOptions {
                r#type: NotifierType::Info,
                position: NotifierPosition::Bottom,
                replace_key: Some("chat-capture-refused".to_string()),
                ..NotifierSendOptions::default()
            },
            cx,
        );
    }

    pub(in super::super) fn handle_capture_hold_cancelled(&mut self, cx: &mut Context<Self>) {
        if std::mem::take(&mut self.capture_refused) {
            self.capture_finger_down = false;
            Self::show_capture_refused(cx);
            cx.notify();
            return;
        }
        self.video_hold_start_task = None;
        self.capture_finger_down = false;
        cx.notify();
        match self.capture_mode {
            ChatComposerCaptureMode::Voice => {
                if self.voice_hold_start_task.take().is_some() {
                    self.voice_recorder.update(cx, |recorder, cx| recorder.cancel_recording(cx));
                    return;
                }
                let past_cancel_line =
                    self.voice_recorder.read(cx).drag_translation().width < px(-VoiceMessageRecorder::CANCEL_SWIPE_THRESHOLD);
                self.voice_recorder.update(cx, |recorder, cx| {
                    if past_cancel_line {
                        let _ = recorder.end_hold(false, cx);
                    } else {
                        recorder.pause_for_background(cx);
                    }
                });
            }
            ChatComposerCaptureMode::VideoNote => {
                let past_cancel_line =
                    self.video_note_recorder.read(cx).drag_translation().width < px(-VideoNoteRecorder::CANCEL_SWIPE_THRESHOLD);
                self.video_note_recorder.update(cx, |recorder, cx| {
                    if past_cancel_line {
                        let _ = recorder.end_hold(false, cx);
                    } else {
                        recorder.pause_for_interruption(cx);
                    }
                });
            }
        }
    }

    pub(in super::super) fn handle_locked_capture_send(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.capture_mode {
            ChatComposerCaptureMode::Voice => {
                let result = self.voice_recorder.update(cx, |recorder, cx| recorder.finish_recording(true, cx));
                self.deliver_voice_recording_result(result, window, cx);
            }
            ChatComposerCaptureMode::VideoNote => {
                self.install_video_note_movie_delegate(window, cx);
                let result = self.video_note_recorder.update(cx, |recorder, cx| recorder.finish_recording(true, cx));
                if !matches!(result, VideoNoteFinishResult::Finishing) {
                    self.deliver_video_note_result(result, window, cx);
                }
            }
        }
        self.capture_finger_down = false;
        cx.notify();
    }

    pub(in super::super) fn handle_locked_capture_cancel(&mut self, cx: &mut Context<Self>) {
        match self.capture_mode {
            ChatComposerCaptureMode::Voice => self.voice_recorder.update(cx, |recorder, cx| recorder.cancel_recording(cx)),
            ChatComposerCaptureMode::VideoNote => self.video_note_recorder.update(cx, |recorder, cx| recorder.cancel_recording(cx)),
        }
        self.capture_finger_down = false;
        cx.notify();
    }

    pub(in super::super) fn handle_capture_pause_or_resume(&mut self, cx: &mut Context<Self>) {
        match self.capture_mode {
            ChatComposerCaptureMode::Voice => self.voice_recorder.update(cx, |recorder, cx| recorder.pause_or_resume(cx)),
            ChatComposerCaptureMode::VideoNote => self.video_note_recorder.update(cx, |recorder, cx| recorder.pause_or_resume(cx)),
        }
    }

    pub(in super::super) fn show_voice_too_short(&mut self, cx: &mut Context<Self>) {
        self.state.voice_too_short_visible = true;
        cx.notify();
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_millis(1600)).await;
            let _ = this.update(cx, |screen, cx| {
                screen.state.voice_too_short_visible = false;
                cx.notify();
            });
        })
        .detach();
    }

    pub(in super::super) fn deliver_video_note_result(&mut self, result: VideoNoteFinishResult, window: &mut Window, cx: &mut Context<Self>) {
        match result {
            VideoNoteFinishResult::Ignored
            | VideoNoteFinishResult::Cancelled
            | VideoNoteFinishResult::Failed
            | VideoNoteFinishResult::Finishing => {}
            VideoNoteFinishResult::TooShort => self.show_voice_too_short(cx),
            VideoNoteFinishResult::Finished { recipe, poster_url, duration_seconds } => {
                let chat_id = self.resolved_chat_id(cx);
                if chat_id.is_empty() {
                    Sai::log("VideoNote", || "deliverVideoNoteResult: send aborted, no chat id".to_owned(), SaiLogKind::Ui, cx);
                    VideoNoteComposition::discard_sources(&recipe);
                    return;
                }
                let caption = trimming_whitespaces_and_newlines(&self.state.input_text).to_owned();
                self.state.input_text = String::new();
                let poster_url = poster_url.and_then(|path| Url::from_file_path(&path));
                let item = MessageInteractionsStore::shared(cx).read(cx).make_video_note_item(
                    &recipe,
                    poster_url,
                    duration_seconds,
                    &caption,
                );
                let Some(item) = item else {
                    VideoNoteComposition::discard_sources(&recipe);
                    return;
                };
                let topic_id = self.resolved_topic_id(cx);
                let post_topic_id = self.post_topic_id(cx);
                let thread_root_id = self.resolved_thread_root_id().map(str::to_owned);
                let reply_id = self.take_reply_target();
                message_interactions(cx, |store, cx| {
                    let _ = store.send_video_note(
                        &chat_id,
                        &item,
                        Some(recipe),
                        &caption,
                        topic_id.as_deref(),
                        post_topic_id.as_deref(),
                        thread_root_id.as_deref(),
                        false,
                        reply_id.as_deref(),
                        None,
                        None,
                        cx,
                    );
                });
                self.follow_outgoing_message(window, cx);
                cx.notify();
            }
        }
    }

    pub(in super::super) fn deliver_voice_recording_result(&mut self, result: VoiceRecordingFinishResult, window: &mut Window, cx: &mut Context<Self>) {
        match result {
            VoiceRecordingFinishResult::Ignored | VoiceRecordingFinishResult::Cancelled | VoiceRecordingFinishResult::Failed => {}
            VoiceRecordingFinishResult::TooShort => self.show_voice_too_short(cx),
            VoiceRecordingFinishResult::Finished { url, metadata } => {
                let chat_id = self.resolved_chat_id(cx);
                if chat_id.is_empty() {
                    return;
                }
                let recorded_bytes = std::fs::metadata(&url).map(|file| file.len()).unwrap_or(0);
                if recorded_bytes == 0 {
                    let _ = std::fs::remove_file(&url);
                    send_failed_notice("chat_media_send_failed", None, cx);
                    return;
                }
                let topic_id = self.resolved_topic_id(cx);
                let post_topic_id = self.post_topic_id(cx);
                let thread_root_id = self.resolved_thread_root_id().map(str::to_owned);
                let reply_id = self.take_reply_target();
                message_interactions(cx, |store, cx| {
                    store.send_voice(
                        &chat_id,
                        &url,
                        &metadata,
                        topic_id.as_deref(),
                        post_topic_id.as_deref(),
                        thread_root_id.as_deref(),
                        reply_id.as_deref(),
                        cx,
                    );
                });
                self.follow_outgoing_message(window, cx);
                cx.notify();
            }
        }
    }

    pub(in super::super) fn park_locked_voice_recording(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let scope_key = self.composer_draft_scope_key(cx);
        let result = self.voice_recorder.update(cx, |recorder, cx| recorder.finish_recording(false, cx));
        let VoiceRecordingFinishResult::Finished { url, metadata } = result else {
            return;
        };
        if self.resolved_chat_id(cx).is_empty()
            || self.channel_posting_locked(cx)
            || metadata.duration_milliseconds < VoiceMessageRecorder::MIN_DURATION_MILLISECONDS
        {
            cx.background_spawn(async move {
                let _ = std::fs::remove_file(url);
            })
            .detach();
            return;
        }
        draft_store(cx, |store, cx| store.set_voice_draft(url, metadata, &scope_key, cx));
    }

    pub(in super::super) fn active_voice_draft(&self, cx: &mut App) -> Option<ChatVoiceDraft> {
        if self.state.edit_target.is_some() || self.channel_posting_locked(cx) {
            return None;
        }
        let scope_key = self.composer_draft_scope_key(cx);
        ChatDraftStore::shared(cx).read(cx).voice_draft(&scope_key)
    }

    pub(in super::super) fn voice_draft_playback_id(&self, cx: &mut App) -> String {
        format!("voice-draft:{}", self.composer_draft_scope_key(cx))
    }

    pub(in super::super) fn toggle_voice_draft_playback(&mut self, cx: &mut Context<Self>) {
        let Some(draft) = self.active_voice_draft(cx) else {
            return;
        };
        let Some(path) = ChatDraftStore::shared(cx).read(cx).voice_draft_file_url(&draft) else {
            return;
        };
        let Some(url) = Url::from_file_path(&path) else {
            return;
        };
        let playback_id = self.voice_draft_playback_id(cx);
        let mut item = MessageMediaItem::new(playback_id.clone(), MessageMediaKind::Voice, url.absolute_string().to_string());
        item.duration = Some(draft.metadata.duration_milliseconds as f64 / 1000.0);
        item.waveform = draft.metadata.waveform.clone();
        ChatVoiceMessagePlayer::shared(cx).update(cx, |player, cx| {
            player.toggle(playback_id, String::new(), String::new(), None, item, cx);
        });
    }

    pub(in super::super) fn stop_voice_draft_playback(&mut self, cx: &mut Context<Self>) {
        let playback_id = self.voice_draft_playback_id(cx);
        let player = ChatVoiceMessagePlayer::shared(cx);
        if player.read(cx).playing_message_id() != Some(playback_id.as_str()) {
            return;
        }
        player.update(cx, |player, cx| player.stop(cx));
    }

    pub(in super::super) fn discard_voice_draft(&mut self, cx: &mut Context<Self>) {
        self.stop_voice_draft_playback(cx);
        let scope_key = self.composer_draft_scope_key(cx);
        draft_store(cx, |store, cx| store.discard_voice_draft(&scope_key, cx));
    }

    pub(in super::super) fn send_voice_draft(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let scope_key = self.composer_draft_scope_key(cx);
        let chat_id = self.resolved_chat_id(cx);
        if self.active_voice_draft(cx).is_none() || chat_id.is_empty() {
            return;
        }
        self.stop_voice_draft_playback(cx);
        let topic_id = self.resolved_topic_id(cx);
        let post_topic_id = self.post_topic_id(cx);
        let thread_root_id = self.resolved_thread_root_id().map(str::to_owned);
        let reply_id = self.take_reply_target();
        let account_user_id = SaiAccountCacheScope::active_user_id();
        let taking = draft_store(cx, |store, cx| store.take_voice_draft(&scope_key, cx));
        let handle = window.window_handle();
        cx.spawn(async move |this, cx| {
            let taken = taking.await;
            let _ = handle.update(cx, |_, window, cx| {
                let Some((url, metadata)) = taken else {
                    if SaiAccountCacheScope::is_current_account(account_user_id.as_deref()) {
                        send_failed_notice("chat_media_send_failed", None, cx);
                    }
                    return;
                };
                message_interactions(cx, |store, cx| {
                    store.send_voice(
                        &chat_id,
                        &url,
                        &metadata,
                        topic_id.as_deref(),
                        post_topic_id.as_deref(),
                        thread_root_id.as_deref(),
                        reply_id.as_deref(),
                        cx,
                    );
                });
                let _ = this.update(cx, |screen, cx| screen.follow_outgoing_message(window, cx));
            });
        })
        .detach();
        cx.notify();
    }
}
