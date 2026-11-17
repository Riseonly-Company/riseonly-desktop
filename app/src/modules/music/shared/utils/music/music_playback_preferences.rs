use gpui::{App, Context, Entity};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_platform::user_defaults::UserDefaults;
use rise_platform::user_defaults_keys::{
    MUSIC_FILE_PLAYBACK_PREFIX, MUSIC_PLAYBACK_PREFIX, MUSIC_PLAYBACK_REPEAT_MODE, MUSIC_PLAYBACK_SHUFFLE,
};

use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::modules::music::shared::utils::music::music_models::MusicRepeatMode;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MusicPlaybackPreferenceScope {
    Music,
    Files,
}

impl MusicPlaybackPreferenceScope {
    pub const ALL_CASES: [Self; 2] = [Self::Music, Self::Files];
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct MusicPlaybackPreferencesSettings {
    repeat_mode: MusicRepeatMode,
    is_shuffled: bool,
}

impl Default for MusicPlaybackPreferencesSettings {
    fn default() -> Self {
        Self {
            repeat_mode: MusicRepeatMode::Off,
            is_shuffled: false,
        }
    }
}

/// device local choices, separate from the room state of a listening session
pub struct MusicPlaybackPreferences {
    music: MusicPlaybackPreferencesSettings,
    files: MusicPlaybackPreferencesSettings,
}

impl MusicPlaybackPreferences {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, |_| {
            let mut preferences = Self {
                music: MusicPlaybackPreferencesSettings::default(),
                files: MusicPlaybackPreferencesSettings::default(),
            };
            preferences.load_account_state();
            preferences
        })
    }

    /// shared store for readers with only &App, none before the first shared call
    pub fn try_read(cx: &App) -> Option<&Self> {
        rise_ui::shared::try_read::<Self>(cx)
    }

    pub fn repeat_mode(&self, scope: MusicPlaybackPreferenceScope) -> MusicRepeatMode {
        self.settings(scope).repeat_mode
    }

    pub fn is_shuffled(&self, scope: MusicPlaybackPreferenceScope) -> bool {
        self.settings(scope).is_shuffled
    }

    pub fn set_repeat_mode(
        &mut self,
        mode: MusicRepeatMode,
        scope: MusicPlaybackPreferenceScope,
        cx: &mut Context<Self>,
    ) {
        let mut next = self.settings(scope);
        next.repeat_mode = mode;
        self.replace(next, scope);
        UserDefaults::standard().set(
            &Self::key(MUSIC_PLAYBACK_REPEAT_MODE, scope, SaiAccountCacheScope::active_user_id().as_deref()),
            mode.wire(),
        );
        cx.notify();
    }

    pub fn set_shuffled(&mut self, enabled: bool, scope: MusicPlaybackPreferenceScope, cx: &mut Context<Self>) {
        let mut next = self.settings(scope);
        next.is_shuffled = enabled;
        self.replace(next, scope);
        UserDefaults::standard().set(
            &Self::key(MUSIC_PLAYBACK_SHUFFLE, scope, SaiAccountCacheScope::active_user_id().as_deref()),
            enabled,
        );
        cx.notify();
    }

    pub fn activate_account_state(&mut self, cx: &mut Context<Self>) {
        self.load_account_state();
        cx.notify();
    }

    pub fn reset_account_state(&mut self, cx: &mut Context<Self>) {
        self.music = MusicPlaybackPreferencesSettings::default();
        self.files = MusicPlaybackPreferencesSettings::default();
        cx.notify();
    }

    pub fn remove_persisted_account(&self, user_id: &str) {
        let defaults = UserDefaults::standard();
        for scope in MusicPlaybackPreferenceScope::ALL_CASES {
            for name in [MUSIC_PLAYBACK_REPEAT_MODE, MUSIC_PLAYBACK_SHUFFLE] {
                defaults.remove_object(&Self::key(name, scope, Some(user_id)));
            }
        }
    }

    fn load_account_state(&mut self) {
        let defaults = UserDefaults::standard();
        let user_id = SaiAccountCacheScope::active_user_id();
        for scope in MusicPlaybackPreferenceScope::ALL_CASES {
            let value = defaults.string(&Self::key(MUSIC_PLAYBACK_REPEAT_MODE, scope, user_id.as_deref()));
            let repeat_mode = value
                .and_then(|raw| MusicRepeatMode::ALL.iter().copied().find(|mode| mode.wire() == raw))
                .unwrap_or(MusicRepeatMode::Off);
            let is_shuffled = defaults.bool(&Self::key(MUSIC_PLAYBACK_SHUFFLE, scope, user_id.as_deref()));
            self.replace(
                MusicPlaybackPreferencesSettings {
                    repeat_mode,
                    is_shuffled,
                },
                scope,
            );
        }
    }

    fn settings(&self, scope: MusicPlaybackPreferenceScope) -> MusicPlaybackPreferencesSettings {
        match scope {
            MusicPlaybackPreferenceScope::Music => self.music,
            MusicPlaybackPreferenceScope::Files => self.files,
        }
    }

    fn replace(&mut self, settings: MusicPlaybackPreferencesSettings, scope: MusicPlaybackPreferenceScope) {
        match scope {
            MusicPlaybackPreferenceScope::Music => self.music = settings,
            MusicPlaybackPreferenceScope::Files => self.files = settings,
        }
    }

    fn key(name: &str, scope: MusicPlaybackPreferenceScope, user_id: Option<&str>) -> String {
        let account = user_id.map(trimming_whitespaces_and_newlines).unwrap_or_default();
        let account = if account.is_empty() { "anonymous" } else { account };
        let playback = match scope {
            MusicPlaybackPreferenceScope::Music => MUSIC_PLAYBACK_PREFIX,
            MusicPlaybackPreferenceScope::Files => MUSIC_FILE_PLAYBACK_PREFIX,
        };
        format!("{}{playback}{name}", SaiAccountCacheScope::prefix(account))
    }
}
