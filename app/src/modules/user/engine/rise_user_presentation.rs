use std::collections::HashSet;

use gpui::{App, Context, Entity};
use rise_core::foundation::foundation_uuid::Uuid;
use rise_theme::{RiseThemePlatform, RiseThemePreferences, ThemeStore};

use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::modules::user::engine::core::rise_user_debug_trace::RiseUserDebugTrace;
use crate::modules::user::engine::rise_user_engine_models::{
    RiseUserThemeSettingsCacheSource, RiseUserThemeSettingsRepositorySnapshot,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RiseUserPresentationSnapshot {
    pub account_id: Option<String>,
    pub account_epoch: Option<String>,
    pub session_id: Option<Uuid>,
    pub generation: u64,
    pub revision: i64,
    pub theme_preferences: RiseThemePreferences,
    pub has_theme_settings: bool,
    pub theme_settings_cache_source: RiseUserThemeSettingsCacheSource,
    pub theme_settings_is_refreshing: bool,
    pub theme_settings_mutating_platforms: HashSet<RiseThemePlatform>,
    pub theme_settings_error: Option<String>,
}

impl RiseUserPresentationSnapshot {
    pub fn empty() -> Self {
        Self {
            account_id: None,
            account_epoch: None,
            session_id: None,
            generation: 0,
            revision: 0,
            theme_preferences: RiseThemePreferences::default(),
            has_theme_settings: false,
            theme_settings_cache_source: RiseUserThemeSettingsCacheSource::Miss,
            theme_settings_is_refreshing: false,
            theme_settings_mutating_platforms: HashSet::new(),
            theme_settings_error: None,
        }
    }
}

pub struct RiseUserPresentation {
    snapshot: RiseUserPresentationSnapshot,
}

impl RiseUserPresentation {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self { snapshot: RiseUserPresentationSnapshot::empty() }
    }

    pub fn snapshot(&self) -> &RiseUserPresentationSnapshot {
        &self.snapshot
    }

    pub fn activate(&mut self, account_id: &str, account_epoch: &str, session_id: Uuid, cx: &mut Context<Self>) {
        if self.snapshot.account_id.as_deref() == Some(account_id)
            && self.snapshot.account_epoch.as_deref() == Some(account_epoch)
            && self.snapshot.session_id == Some(session_id)
        {
            return;
        }
        let activated = account_id.to_owned();
        ThemeStore::shared(cx).update(cx, |store, cx| store.activate(Some(activated), cx));
        self.snapshot = RiseUserPresentationSnapshot {
            account_id: Some(account_id.to_owned()),
            account_epoch: Some(account_epoch.to_owned()),
            session_id: Some(session_id),
            generation: self.snapshot.generation.wrapping_add(1),
            ..RiseUserPresentationSnapshot::empty()
        };
        cx.notify();
    }

    pub fn publish(&mut self, value: &RiseUserThemeSettingsRepositorySnapshot, cx: &mut Context<Self>) -> bool {
        if self.snapshot.account_id.as_deref() != Some(value.account_id.as_str())
            || self.snapshot.account_epoch.as_deref() != Some(value.account_epoch.as_str())
            || self.snapshot.session_id != Some(value.session_id)
        {
            return false;
        }
        self.snapshot = RiseUserPresentationSnapshot {
            account_id: Some(value.account_id.clone()),
            account_epoch: Some(value.account_epoch.clone()),
            session_id: Some(value.session_id),
            generation: self.snapshot.generation.wrapping_add(1),
            revision: value.revision,
            theme_preferences: value.preferences,
            has_theme_settings: value.has_value,
            theme_settings_cache_source: value.cache_source,
            theme_settings_is_refreshing: value.is_refreshing,
            theme_settings_mutating_platforms: value.mutating_platforms.clone(),
            theme_settings_error: value.error_code.clone(),
        };
        if value.has_value {
            let preferences = value.preferences;
            let account_id = value.account_id.clone();
            ThemeStore::shared(cx).update(cx, |store, cx| {
                let _ = store.install(preferences, &account_id, cx);
            });
        }
        cx.notify();
        true
    }

    pub fn apply_local_choice(&mut self, preferences: RiseThemePreferences, account_id: &str, cx: &mut Context<Self>) {
        if self.snapshot.account_id.as_deref() != Some(account_id) {
            return;
        }
        let generation = self.snapshot.generation.wrapping_add(1);
        self.snapshot = RiseUserPresentationSnapshot {
            generation,
            theme_preferences: preferences,
            has_theme_settings: true,
            ..self.snapshot.clone()
        };
        cx.notify();
    }

    pub fn deactivate(&mut self, account_id: &str, session_id: Uuid, cx: &mut Context<Self>) {
        if self.snapshot.account_id.as_deref() != Some(account_id) || self.snapshot.session_id != Some(session_id) {
            return;
        }
        ThemeStore::shared(cx).update(cx, |store, cx| {
            let _ = store.deactivate(account_id, cx);
        });
        self.snapshot = RiseUserPresentationSnapshot {
            generation: self.snapshot.generation.wrapping_add(1),
            ..RiseUserPresentationSnapshot::empty()
        };
        cx.notify();
    }

    pub fn report_theme_settings_failure(
        &mut self,
        error: &anyhow::Error,
        account_id: Option<&str>,
        account_epoch: Option<&str>,
        cx: &mut Context<Self>,
    ) {
        let (Some(account_id), Some(account_epoch)) = (account_id, account_epoch) else {
            return;
        };
        if !SaiAccountCacheScope::is_current_account_epoch(Some(account_id), Some(account_epoch)) {
            return;
        }
        if let Some(snapshot_account_id) = self.snapshot.account_id.as_deref()
            && snapshot_account_id != account_id
        {
            return;
        }
        let activated = account_id.to_owned();
        ThemeStore::shared(cx).update(cx, |store, cx| store.activate(Some(activated), cx));
        self.snapshot = RiseUserPresentationSnapshot {
            account_id: Some(account_id.to_owned()),
            account_epoch: Some(account_epoch.to_owned()),
            session_id: self.snapshot.session_id,
            generation: self.snapshot.generation.wrapping_add(1),
            revision: self.snapshot.revision,
            theme_preferences: self.snapshot.theme_preferences,
            has_theme_settings: self.snapshot.has_theme_settings,
            theme_settings_cache_source: self.snapshot.theme_settings_cache_source,
            theme_settings_is_refreshing: false,
            theme_settings_mutating_platforms: self.snapshot.theme_settings_mutating_platforms.clone(),
            theme_settings_error: Some(RiseUserDebugTrace::error_label(error)),
        };
        cx.notify();
    }
}
