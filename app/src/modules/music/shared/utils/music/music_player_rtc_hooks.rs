use gpui::App;

use crate::core::feature_hooks::FeatureHookSlot;

pub struct MusicPlayerRtcHooks {
    pub request_music_playback: fn(retry: Box<dyn FnOnce(&mut App)>, cx: &mut App) -> bool,
}

pub(crate) static HOOKS: FeatureHookSlot<MusicPlayerRtcHooks> = FeatureHookSlot::new("HOOKS");

static UNINSTALLED: MusicPlayerRtcHooks = MusicPlayerRtcHooks {
    request_music_playback: |_, _| true,
};

/// installs the hooks whole
/// second call is refused and hands them back, first installer wins
pub fn set_music_player_rtc_hooks(hooks: MusicPlayerRtcHooks) -> Result<(), MusicPlayerRtcHooks> {
    HOOKS.set(hooks)
}

pub fn music_player_rtc_hooks() -> &'static MusicPlayerRtcHooks {
    HOOKS.get().unwrap_or(&UNINSTALLED)
}
