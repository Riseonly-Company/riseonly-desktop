use gpui::App;
use rise_core::foundation::foundation_uuid::Uuid;

use crate::core::feature_hooks::FeatureHookSlot;

/// the chat voice message player as the music player sees it
pub struct MusicPlayerStateVoiceHooks {
    pub is_active: fn(&mut App) -> bool,
    pub stop: fn(&mut App),
}

/// the chat video note player as the music player sees it
pub struct MusicPlayerStateVideoNoteHooks {
    pub is_active: fn(&mut App) -> bool,
    pub stop: fn(&mut App),
}

/// where playing music counts as meaningful activity
pub struct MusicPlayerStateActivityHooks {
    pub set_source: fn(Uuid, bool, Box<dyn Fn(&App) -> bool>, &mut App),
}

pub(crate) static MUSIC_PLAYER_STATE_VOICE_HOOKS: FeatureHookSlot<MusicPlayerStateVoiceHooks> =
    FeatureHookSlot::new("MUSIC_PLAYER_STATE_VOICE_HOOKS");
pub(crate) static MUSIC_PLAYER_STATE_VIDEO_NOTE_HOOKS: FeatureHookSlot<MusicPlayerStateVideoNoteHooks> =
    FeatureHookSlot::new("MUSIC_PLAYER_STATE_VIDEO_NOTE_HOOKS");
pub(crate) static MUSIC_PLAYER_STATE_ACTIVITY_HOOKS: FeatureHookSlot<MusicPlayerStateActivityHooks> =
    FeatureHookSlot::new("MUSIC_PLAYER_STATE_ACTIVITY_HOOKS");

pub fn set_music_player_state_voice_hooks(hooks: MusicPlayerStateVoiceHooks) {
    let _ = MUSIC_PLAYER_STATE_VOICE_HOOKS.set(hooks);
}

pub fn music_player_state_voice_hooks() -> Option<&'static MusicPlayerStateVoiceHooks> {
    MUSIC_PLAYER_STATE_VOICE_HOOKS.get()
}

pub fn set_music_player_state_video_note_hooks(hooks: MusicPlayerStateVideoNoteHooks) {
    let _ = MUSIC_PLAYER_STATE_VIDEO_NOTE_HOOKS.set(hooks);
}

pub fn music_player_state_video_note_hooks() -> Option<&'static MusicPlayerStateVideoNoteHooks> {
    MUSIC_PLAYER_STATE_VIDEO_NOTE_HOOKS.get()
}

pub fn set_music_player_state_activity_hooks(hooks: MusicPlayerStateActivityHooks) {
    let _ = MUSIC_PLAYER_STATE_ACTIVITY_HOOKS.set(hooks);
}

pub fn music_player_state_activity_hooks() -> Option<&'static MusicPlayerStateActivityHooks> {
    MUSIC_PLAYER_STATE_ACTIVITY_HOOKS.get()
}
