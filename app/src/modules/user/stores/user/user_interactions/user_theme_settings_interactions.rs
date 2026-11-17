use gpui::Context;
use rise_canon::RiseGuideSignal;
use rise_theme::{RiseThemeAppearance, RiseThemePlatform, ThemeStore};

use crate::core::guides::guide_signal_bus::GuideSignalBus;
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::user::stores::user::user_interactions::user_interactions::UserInteractionsStore;
use crate::modules::user::stores::user::user_services::user_guide_sandbox_hooks::user_guide_sandbox_hooks;

impl UserInteractionsStore {
    pub fn set_launch_animation_enabled(&mut self, enabled: bool, cx: &mut Context<Self>) {
        Self::actions(cx).update(cx, |actions, cx| actions.set_launch_animation_enabled_action(enabled, cx));
    }

    pub fn load_theme_settings(&mut self, force: bool, cx: &mut Context<Self>) {
        if !AuthServicesStore::shared(cx).read(cx).is_authenticated() {
            return;
        }
        Self::actions(cx).update(cx, |actions, cx| actions.load_theme_settings_action(force, cx));
    }

    pub fn set_theme_appearance(
        &mut self,
        appearance: Option<RiseThemeAppearance>,
        platform: RiseThemePlatform,
        cx: &mut Context<Self>,
    ) {
        if !AuthServicesStore::shared(cx).read(cx).is_authenticated() {
            return;
        }
        if (user_guide_sandbox_hooks().owns_theme_scene)(cx) {
            if let Some(account_id) = SaiAccountCacheScope::active_user_id() {
                ThemeStore::shared(cx).update(cx, |theme, cx| {
                    let preferences = theme.preferences().setting(appearance, platform);
                    let _ = theme.install(preferences, &account_id, cx);
                });
            }
            GuideSignalBus::emit(RiseGuideSignal::ThemeAppearanceChanged);
            return;
        }
        Self::actions(cx).update(cx, |actions, cx| actions.set_theme_appearance_action(appearance, platform, cx));
    }
}
