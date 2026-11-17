use std::cell::RefCell;

use gpui::App;

use crate::modules::chat::shared::chat_music_hooks::{ChatMusicCallback, ChatMusicHooks, set_chat_music_hooks};
use crate::modules::music::shared::utils::music::music_external_audio_owner::MusicExternalAudioOwner;
use crate::modules::music::shared::utils::music::music_listening_interruption_gate::MusicListeningInterruptionGate;
use crate::modules::music::shared::utils::music::music_player_state::MusicPlayerState;

pub(crate) fn install_chat_music_hooks() {
    set_chat_music_hooks(ChatMusicHooks {
        suspend_for_external_audio: |owner, resume_when_finished, cx| {
            MusicPlayerState::shared(cx)
                .update(cx, |state, cx| state.suspend_for_external_audio(owner, resume_when_finished, cx));
        },
        resume_after_external_audio: |owner, cx| {
            MusicPlayerState::shared(cx).update(cx, |state, cx| state.resume_after_external_audio(owner, cx));
        },
        is_external_audio_owner_active: |owner: MusicExternalAudioOwner, cx: &App| {
            MusicPlayerState::try_read(cx).is_some_and(|state| state.is_external_audio_owner_active(&owner))
        },
        request_external_playback: |token, autoplay, retry: ChatMusicCallback, on_cancel, cx| {
            MusicListeningInterruptionGate::shared(cx)
                .update(cx, |gate, cx| gate.request_external_playback(token, autoplay, retry, on_cancel, cx))
        },
        playback_scope_id: |cx| MusicPlayerState::try_read(cx).and_then(|state| state.playback_scope_id.clone()),
        source_queue: |cx| MusicPlayerState::try_read(cx).map(|state| state.source_queue().to_vec()).unwrap_or_default(),
        current_track: |cx| MusicPlayerState::try_read(cx).and_then(|state| state.current_track.clone()),
        has_listening_session: |cx| MusicPlayerState::try_read(cx).is_some_and(|state| state.listening_session.is_some()),
        replace_ad_hoc_queue: |scope_id, tracks, cx| {
            MusicPlayerState::shared(cx).update(cx, |state, cx| state.replace_ad_hoc_queue(scope_id, tracks, cx))
        },
        observe_playback: |on_change, cx| {
            let state = MusicPlayerState::shared(cx);
            let tracked = |state: &MusicPlayerState| {
                (
                    state.current_track.as_ref().map(|current| current.id.clone()),
                    state.playback_scope_id.clone(),
                    state.listening_session.is_none(),
                    state.source_queue().len(),
                )
            };
            let last = RefCell::new(tracked(state.read(cx)));
            cx.observe(&state, move |state, cx| {
                let now = tracked(state.read(cx));
                if *last.borrow() == now {
                    return;
                }
                *last.borrow_mut() = now;
                on_change(cx);
            })
        },
    });
}
