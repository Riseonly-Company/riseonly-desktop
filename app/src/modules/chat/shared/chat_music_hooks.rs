use std::rc::Rc;

use gpui::{App, Subscription};

use crate::core::feature_hooks::FeatureHookSlot;
use crate::modules::music::shared::utils::music::music_external_audio_owner::MusicExternalAudioOwner;
use crate::modules::music::shared::utils::music::music_models::TrackModel;

/// retry and cancel closures of the external playback request
pub(crate) type ChatMusicCallback = Rc<dyn Fn(&mut App)>;

pub(crate) struct ChatMusicHooks {
    pub suspend_for_external_audio: fn(owner: MusicExternalAudioOwner, resume_when_finished: bool, cx: &mut App),
    pub resume_after_external_audio: fn(owner: MusicExternalAudioOwner, cx: &mut App),
    pub is_external_audio_owner_active: fn(owner: MusicExternalAudioOwner, cx: &App) -> bool,
    pub request_external_playback: fn(
        token: Option<MusicExternalAudioOwner>,
        autoplay: bool,
        retry: ChatMusicCallback,
        on_cancel: Option<ChatMusicCallback>,
        cx: &mut App,
    ) -> bool,
    pub playback_scope_id: fn(cx: &App) -> Option<String>,
    pub source_queue: fn(cx: &App) -> Vec<TrackModel>,
    pub current_track: fn(cx: &App) -> Option<TrackModel>,
    pub has_listening_session: fn(cx: &App) -> bool,
    pub replace_ad_hoc_queue: fn(scope_id: &str, tracks: Vec<TrackModel>, cx: &mut App) -> bool,
    pub observe_playback: fn(on_change: Box<dyn Fn(&mut App)>, cx: &mut App) -> Subscription,
}

pub(crate) static CHAT_MUSIC_HOOKS: FeatureHookSlot<ChatMusicHooks> = FeatureHookSlot::new("CHAT_MUSIC_HOOKS");

/// installs hooks once, second call keeps the first hooks
pub(crate) fn set_chat_music_hooks(hooks: ChatMusicHooks) {
    let _ = CHAT_MUSIC_HOOKS.set(hooks);
}

pub(crate) fn chat_music_hooks() -> Option<&'static ChatMusicHooks> {
    CHAT_MUSIC_HOOKS.get()
}

impl ChatMusicHooks {
    pub(crate) fn suspend_for_external_audio(
        owner: MusicExternalAudioOwner,
        resume_when_finished: bool,
        cx: &mut App,
    ) {
        if let Some(hooks) = chat_music_hooks() {
            (hooks.suspend_for_external_audio)(owner, resume_when_finished, cx);
        }
    }

    pub(crate) fn resume_after_external_audio(owner: MusicExternalAudioOwner, cx: &mut App) {
        if let Some(hooks) = chat_music_hooks() {
            (hooks.resume_after_external_audio)(owner, cx);
        }
    }

    pub(crate) fn is_external_audio_owner_active(owner: MusicExternalAudioOwner, cx: &App) -> bool {
        chat_music_hooks().is_some_and(|hooks| (hooks.is_external_audio_owner_active)(owner, cx))
    }

    /// without music nothing listens so playback may start right now
    pub(crate) fn request_external_playback(
        token: Option<MusicExternalAudioOwner>,
        autoplay: bool,
        retry: ChatMusicCallback,
        on_cancel: Option<ChatMusicCallback>,
        cx: &mut App,
    ) -> bool {
        match chat_music_hooks() {
            Some(hooks) => (hooks.request_external_playback)(token, autoplay, retry, on_cancel, cx),
            None => true,
        }
    }

    pub(crate) fn playback_scope_id(cx: &App) -> Option<String> {
        chat_music_hooks().and_then(|hooks| (hooks.playback_scope_id)(cx))
    }

    pub(crate) fn source_queue(cx: &App) -> Vec<TrackModel> {
        chat_music_hooks().map_or_else(Vec::new, |hooks| (hooks.source_queue)(cx))
    }

    pub(crate) fn current_track(cx: &App) -> Option<TrackModel> {
        chat_music_hooks().and_then(|hooks| (hooks.current_track)(cx))
    }

    pub(crate) fn has_listening_session(cx: &App) -> bool {
        chat_music_hooks().is_some_and(|hooks| (hooks.has_listening_session)(cx))
    }

    pub(crate) fn replace_ad_hoc_queue(scope_id: &str, tracks: Vec<TrackModel>, cx: &mut App) -> bool {
        chat_music_hooks().is_some_and(|hooks| (hooks.replace_ad_hoc_queue)(scope_id, tracks, cx))
    }

    /// none while no music is installed, nothing can change so nothing to observe
    pub(crate) fn observe_playback(on_change: Box<dyn Fn(&mut App)>, cx: &mut App) -> Option<Subscription> {
        chat_music_hooks().map(|hooks| (hooks.observe_playback)(on_change, cx))
    }
}
