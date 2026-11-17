use std::time::Duration;

use gpui::{App, Context, Entity, Task};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_platform::user_defaults::UserDefaults;
use rise_platform::user_defaults_keys::{
    MUSIC_AUDIO_EFFECT_ENABLED_SUFFIX, MUSIC_AUDIO_EFFECT_PREFIX, MUSIC_AUDIO_EFFECT_VALUE_SUFFIX,
};

use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::modules::music::shared::utils::music::music_audio_effect_settings::{
    MusicAudioEffectKind, MusicAudioEffectKindExt, MusicAudioEffectSettings,
};

/// device local prefs for the playback effects, not a server owned resource
/// an active listening session carries its own tempo
/// slider positions persist while the effect is off
pub struct MusicAudioEffectStore {
    settings: MusicAudioEffectSettings,
    persist_task: Option<Task<()>>,
    is_hydrating: bool,
    account_user_id: Option<String>,
    account_epoch: Option<String>,
}

impl MusicAudioEffectStore {
    const PERSIST_COALESCING: Duration = Duration::from_millis(400);

    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, |_| {
            let mut store = Self {
                settings: MusicAudioEffectSettings::off(),
                persist_task: None,
                is_hydrating: true,
                account_user_id: None,
                account_epoch: None,
            };
            store.load_current_account_state();
            store.is_hydrating = false;
            store
        })
    }

    pub fn settings(&self) -> &MusicAudioEffectSettings {
        &self.settings
    }

    pub fn is_enabled(&self) -> bool {
        self.settings.is_enabled(MusicAudioEffectKind::Slowed)
    }

    pub fn rate(&self) -> f64 {
        self.settings.value(MusicAudioEffectKind::Slowed)
    }

    pub fn set_enabled(&mut self, kind: MusicAudioEffectKind, value: bool, cx: &mut Context<Self>) {
        let mut next = self.settings.clone();
        next.set_enabled(kind, value);
        self.commit(next, cx);
    }

    pub fn set_value(&mut self, kind: MusicAudioEffectKind, value: f64, cx: &mut Context<Self>) {
        let mut next = self.settings.clone();
        next.set_value(kind, value);
        self.commit(next, cx);
    }

    pub fn replace(&mut self, next: MusicAudioEffectSettings, cx: &mut Context<Self>) {
        self.commit(next, cx);
    }

    pub fn reset_account_state(&mut self, cx: &mut Context<Self>) {
        self.flush_pending_persist();
        self.is_hydrating = true;
        self.settings = MusicAudioEffectSettings::off();
        self.account_user_id = None;
        self.account_epoch = None;
        self.is_hydrating = false;
        cx.notify();
    }

    pub fn activate_account_state(&mut self, cx: &mut Context<Self>) {
        self.flush_pending_persist();
        self.is_hydrating = true;
        self.load_current_account_state();
        self.is_hydrating = false;
        cx.notify();
    }

    pub fn remove_persisted_account(&self, user_id: &str) {
        let defaults = UserDefaults::standard();
        for kind in MusicAudioEffectKind::all_cases() {
            defaults.remove_object(&Self::enabled_key(*kind, Some(user_id)));
            defaults.remove_object(&Self::value_key(*kind, Some(user_id)));
        }
    }

    fn commit(&mut self, next: MusicAudioEffectSettings, cx: &mut Context<Self>) {
        if !SaiAccountCacheScope::is_current_account_epoch(
            self.account_user_id.as_deref(),
            self.account_epoch.as_deref(),
        ) || next == self.settings
        {
            return;
        }
        self.settings = next;
        self.schedule_persist(cx);
        cx.notify();
    }

    fn load_current_account_state(&mut self) {
        self.account_user_id = SaiAccountCacheScope::active_user_id();
        self.account_epoch = SaiAccountCacheScope::active_epoch();
        let defaults = UserDefaults::standard();
        let user_id = self.account_user_id.as_deref();
        let mut loaded = MusicAudioEffectSettings::off();
        for kind in MusicAudioEffectKind::all_cases() {
            loaded.set_enabled(*kind, defaults.bool(&Self::enabled_key(*kind, user_id)));
            let key = Self::value_key(*kind, user_id);
            if defaults.object(&key).is_some() {
                loaded.set_value(*kind, defaults.double(&key));
            } else {
                loaded.set_value(*kind, kind.default_value());
            }
        }
        self.settings = loaded;
    }

    fn schedule_persist(&mut self, cx: &mut Context<Self>) {
        if self.is_hydrating {
            return;
        }
        let snapshot = self.settings.clone();
        let user_id = self.account_user_id.clone();
        self.persist_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Self::PERSIST_COALESCING).await;
            Self::persist(&snapshot, user_id.as_deref());
            let _ = this.update(cx, |store, _| store.persist_task = None);
        }));
    }

    fn flush_pending_persist(&mut self) {
        if self.persist_task.take().is_none() {
            return;
        }
        Self::persist(&self.settings, self.account_user_id.as_deref());
    }

    fn persist(snapshot: &MusicAudioEffectSettings, user_id: Option<&str>) {
        let defaults = UserDefaults::standard();
        for kind in MusicAudioEffectKind::all_cases() {
            defaults.set(&Self::enabled_key(*kind, user_id), snapshot.is_enabled(*kind));
            defaults.set(&Self::value_key(*kind, user_id), snapshot.value(*kind));
        }
    }

    fn enabled_key(kind: MusicAudioEffectKind, user_id: Option<&str>) -> String {
        Self::scoped_key(
            &format!("{MUSIC_AUDIO_EFFECT_PREFIX}{}{MUSIC_AUDIO_EFFECT_ENABLED_SUFFIX}", kind.raw_value()),
            user_id,
        )
    }

    fn value_key(kind: MusicAudioEffectKind, user_id: Option<&str>) -> String {
        Self::scoped_key(
            &format!("{MUSIC_AUDIO_EFFECT_PREFIX}{}{MUSIC_AUDIO_EFFECT_VALUE_SUFFIX}", kind.raw_value()),
            user_id,
        )
    }

    fn scoped_key(key: &str, user_id: Option<&str>) -> String {
        let value = user_id.map(trimming_whitespaces_and_newlines).unwrap_or_default();
        let account = if value.is_empty() { "anonymous" } else { value };
        SaiAccountCacheScope::prefix(account) + key
    }
}
