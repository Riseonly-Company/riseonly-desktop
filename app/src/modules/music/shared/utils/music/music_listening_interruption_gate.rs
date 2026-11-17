use std::rc::Rc;

use gpui::Styled;
use gpui::{
    App, Context, Entity, IntoElement, PromptButton, PromptLevel, Render, Subscription, Window,
    div,
};
use rise_core::foundation::foundation_uuid::Uuid;
use rise_i18n::tr;

use crate::modules::music::shared::utils::music::music_external_audio_owner::MusicExternalAudioOwner;
use crate::modules::music::shared::utils::music::music_player_state::MusicPlayerState;

/// what playing other media means for the active listening session
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MusicListeningInterruptionDecision {
    Proceed,
    ConfirmPauseAll,
    ConfirmLeave,
}

/// automatic enforcement when audio is taken away with no user choice
/// nobody to ask so it applies directly
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MusicListeningAutoInterruptionAction {
    None,
    PauseAll,
    Leave,
}

/// what to do when playback recovers after a stall while a session is attached
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MusicListeningResyncAction {
    Republish,
    Align,
}

pub enum MusicListeningInterruptionPolicy {}

impl MusicListeningInterruptionPolicy {
    /// true when starting other media needs a confirm first
    /// alert only fires if theres something to pause and no consent was given this episode
    pub fn decision(
        has_session: bool,
        is_host: bool,
        can_control_playback: bool,
        session_is_playing: bool,
        local_is_playing: bool,
        consent_episode_active: bool,
    ) -> MusicListeningInterruptionDecision {
        if !has_session {
            return MusicListeningInterruptionDecision::Proceed;
        }
        if !(is_host || can_control_playback) {
            return MusicListeningInterruptionDecision::ConfirmLeave;
        }
        if consent_episode_active {
            return MusicListeningInterruptionDecision::Proceed;
        }
        if session_is_playing || local_is_playing {
            MusicListeningInterruptionDecision::ConfirmPauseAll
        } else {
            MusicListeningInterruptionDecision::Proceed
        }
    }

    pub fn auto_action(
        has_session: bool,
        is_host: bool,
        can_control_playback: bool,
        session_is_playing: bool,
        local_was_playing: bool,
        already_paused_by_local: bool,
    ) -> MusicListeningAutoInterruptionAction {
        if !has_session {
            return MusicListeningAutoInterruptionAction::None;
        }
        if !(session_is_playing || local_was_playing) {
            return MusicListeningAutoInterruptionAction::None;
        }
        if !(is_host || can_control_playback) {
            return MusicListeningAutoInterruptionAction::Leave;
        }
        if already_paused_by_local {
            return MusicListeningAutoInterruptionAction::None;
        }
        MusicListeningAutoInterruptionAction::PauseAll
    }

    /// paused_by_local means this device paused the room after losing audio
    /// if the room is still paused it resumes it, if someone else resumed it takes the room state instead
    pub fn resync_action(is_host: bool, paused_by_local: bool, session_is_playing: bool) -> MusicListeningResyncAction {
        if paused_by_local {
            return if session_is_playing {
                MusicListeningResyncAction::Align
            } else {
                MusicListeningResyncAction::Republish
            };
        }
        if is_host {
            MusicListeningResyncAction::Republish
        } else {
            MusicListeningResyncAction::Align
        }
    }
}

/// retry and cancel closures of a gated playback request
pub type MusicListeningInterruptionCallback = Rc<dyn Fn(&mut App)>;

#[derive(Clone)]
pub struct MusicListeningInterruptionGatePrompt {
    pub id: Uuid,
    pub decision: MusicListeningInterruptionDecision,
    pub token: Option<MusicExternalAudioOwner>,
    pub on_continue: MusicListeningInterruptionCallback,
    pub on_cancel: Option<MusicListeningInterruptionCallback>,
}

/// single confirm point for every surface that would displace the listening session
/// one alert at a time, continue applies the consequence then retries playback
pub struct MusicListeningInterruptionGate {
    prompt: Option<MusicListeningInterruptionGatePrompt>,
    autoplay_declined_session_id: Option<String>,
}

impl MusicListeningInterruptionGate {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, |_| Self {
            prompt: None,
            autoplay_declined_session_id: None,
        })
    }

    pub fn prompt(&self) -> Option<&MusicListeningInterruptionGatePrompt> {
        self.prompt.as_ref()
    }

    pub fn allows_external_audio_now(&self, cx: &mut Context<Self>) -> bool {
        self.prompt.is_none() && Self::current_decision(cx) == MusicListeningInterruptionDecision::Proceed
    }

    /// true when playback may start now, false after showing the alert
    /// continue runs retry, cancel runs on_cancel
    /// token gets invalidated on teardown so a stale retry cant fire
    pub fn request_external_playback(
        &mut self,
        token: Option<MusicExternalAudioOwner>,
        autoplay: bool,
        retry: MusicListeningInterruptionCallback,
        on_cancel: Option<MusicListeningInterruptionCallback>,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.prompt.is_some() {
            return false;
        }
        let decision = Self::current_decision(cx);
        if decision == MusicListeningInterruptionDecision::Proceed {
            return true;
        }
        let session_id = MusicPlayerState::shared(cx)
            .read(cx)
            .listening_session()
            .map(|session| session.session_id.clone());
        if autoplay
            && let Some(session_id) = session_id.as_ref()
            && self.autoplay_declined_session_id.as_ref() == Some(session_id)
        {
            return false;
        }

        let gate = cx.weak_entity();
        let on_continue: MusicListeningInterruptionCallback = Rc::new({
            let gate = gate.clone();
            move |cx: &mut App| {
                let _ = gate.update(cx, |gate, cx| {
                    gate.prompt = None;
                    cx.notify();
                });
                MusicPlayerState::shared(cx)
                    .update(cx, |player, cx| player.apply_consented_listening_interruption(decision, cx));
                retry(cx);
            }
        });
        let cancel: MusicListeningInterruptionCallback = Rc::new(move |cx: &mut App| {
            let _ = gate.update(cx, |gate, cx| {
                gate.prompt = None;
                gate.autoplay_declined_session_id = session_id.clone();
                cx.notify();
            });
            if let Some(on_cancel) = on_cancel.as_ref() {
                on_cancel(cx);
            }
        });
        self.prompt = Some(MusicListeningInterruptionGatePrompt {
            id: Uuid::new_v4(),
            decision,
            token,
            on_continue,
            on_cancel: Some(cancel),
        });
        cx.notify();
        false
    }

    pub fn cancel_prompt(&mut self, cx: &mut Context<Self>) {
        let cancelled = self.prompt.take();
        cx.notify();
        if let Some(on_cancel) = cancelled.and_then(|prompt| prompt.on_cancel) {
            cx.defer(move |cx| on_cancel(cx));
        }
    }

    /// surface is gone so drop its pending prompt without running either callback
    pub fn invalidate_prompt(&mut self, token: MusicExternalAudioOwner, cx: &mut Context<Self>) {
        if self.prompt.as_ref().and_then(|prompt| prompt.token.as_ref()) != Some(&token) {
            return;
        }
        self.prompt = None;
        cx.notify();
    }

    fn current_decision(cx: &mut App) -> MusicListeningInterruptionDecision {
        let player = MusicPlayerState::shared(cx);
        let player = player.read(cx);
        if !player.is_listening_together() {
            return MusicListeningInterruptionDecision::Proceed;
        }
        let Some(session) = player.listening_session() else {
            return MusicListeningInterruptionDecision::Proceed;
        };
        if player.is_listening_playback_displaced_by_ad_hoc() {
            return MusicListeningInterruptionDecision::Proceed;
        }
        MusicListeningInterruptionPolicy::decision(
            true,
            player.is_listening_session_host(),
            session.viewer_permissions.can_control_playback,
            session.is_playing,
            player.is_playing,
            player.listening_consent_episode_active(),
        )
    }
}

/// invisible host for the gate alert, mounted once at the root so it shows above everything
pub struct MusicListeningInterruptionAlertHost {
    gate: Entity<MusicListeningInterruptionGate>,
    presented_prompt_id: Option<Uuid>,
    _gate_observation: Subscription,
}

impl MusicListeningInterruptionAlertHost {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let gate = MusicListeningInterruptionGate::shared(cx);
        let gate_observation = cx.observe(&gate, |_, _, cx| cx.notify());
        Self {
            gate,
            presented_prompt_id: None,
            _gate_observation: gate_observation,
        }
    }

    fn present(&mut self, prompt: MusicListeningInterruptionGatePrompt, window: &mut Window, cx: &mut Context<Self>) {
        self.presented_prompt_id = Some(prompt.id);
        let (title, message) = if prompt.decision == MusicListeningInterruptionDecision::ConfirmLeave {
            (tr("music_live_interrupt_leave_title"), tr("music_live_interrupt_leave_message"))
        } else {
            (tr("music_live_interrupt_pause_all_title"), tr("music_live_interrupt_pause_all_message"))
        };
        let answer = window.prompt(
            PromptLevel::Info,
            &title,
            Some(message.as_str()),
            &[
                PromptButton::ok(tr("music_live_interrupt_continue")),
                PromptButton::cancel(tr("cancel")),
            ],
            cx,
        );
        let gate = self.gate.clone();
        let prompt_id = prompt.id;
        cx.spawn(async move |this, cx| {
            let answer = answer.await;
            let _ = this.update(cx, |host, _| {
                if host.presented_prompt_id == Some(prompt_id) {
                    host.presented_prompt_id = None;
                }
            });
            let _ = gate.update(cx, |gate, cx| {
                if gate.prompt.as_ref().map(|current| current.id) != Some(prompt_id) {
                    return;
                }
                if matches!(answer, Ok(0)) {
                    let on_continue = prompt.on_continue.clone();
                    cx.defer(move |cx| on_continue(cx));
                } else {
                    gate.cancel_prompt(cx);
                }
            });
        })
        .detach();
    }
}

impl Render for MusicListeningInterruptionAlertHost {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let pending = self.gate.read(cx).prompt().cloned();
        match pending {
            Some(prompt) if self.presented_prompt_id != Some(prompt.id) => {
                cx.defer_in(window, move |host, window, cx| host.present(prompt, window, cx));
            }
            None => self.presented_prompt_id = None,
            Some(_) => {}
        }
        div().size_0()
    }
}
