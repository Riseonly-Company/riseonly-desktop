use gpui::App;
use rise_theme::{RiseThemePlatform, RiseThemePreferences};

use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::preferences::rise_launch_preferences::RiseLaunchPreferences;
use crate::modules::user::engine::rise_user_presentation::{RiseUserPresentation, RiseUserPresentationSnapshot};
use crate::modules::user::stores::user::user_services::user_services::UserServicesStore;

impl UserServicesStore {
    pub fn launch_animation_enabled(&self, cx: &mut App) -> bool {
        RiseLaunchPreferences::shared(cx).read(cx).animation_enabled()
    }

    fn user_presentation_snapshot(cx: &App) -> RiseUserPresentationSnapshot {
        rise_ui::shared::try_read::<RiseUserPresentation>(cx)
            .map(|presentation| presentation.snapshot().clone())
            .unwrap_or_else(RiseUserPresentationSnapshot::empty)
    }

    pub fn account_session_is_published(&self, cx: &App) -> bool {
        let snapshot = Self::user_presentation_snapshot(cx);
        let Some(account_id) = snapshot.account_id else {
            return false;
        };
        Some(account_id) == SaiAccountCacheScope::active_user_id()
            && snapshot.account_epoch == SaiAccountCacheScope::active_epoch()
    }

    pub fn theme_settings_has_value(&self, cx: &App) -> bool {
        Self::user_presentation_snapshot(cx).has_theme_settings
    }

    pub fn theme_preferences(&self, cx: &App) -> RiseThemePreferences {
        Self::user_presentation_snapshot(cx).theme_preferences
    }

    pub fn theme_settings_error(&self, cx: &App) -> Option<String> {
        Self::user_presentation_snapshot(cx).theme_settings_error
    }

    pub fn theme_settings_is_refreshing(&self, cx: &App) -> bool {
        Self::user_presentation_snapshot(cx).theme_settings_is_refreshing
    }

    pub fn theme_settings_is_mutating(&self, platform: RiseThemePlatform, cx: &App) -> bool {
        Self::user_presentation_snapshot(cx)
            .theme_settings_mutating_platforms
            .contains(&platform)
    }
}
