use gpui::{App, AsyncApp, Context};
use rise_theme::{RiseThemeAppearance, RiseThemePlatform, ThemeStore};

use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::lib::sai::sai::Sai;
use crate::core::preferences::rise_launch_preferences::RiseLaunchPreferences;
use crate::modules::user::engine::core::rise_user_debug_trace::RiseUserDebugTrace;
use crate::modules::user::engine::rise_user_domain::RiseUserActionExecutor;
use crate::modules::user::engine::rise_user_presentation::RiseUserPresentation;
use crate::modules::user::engine::rise_user_settings_repository::RiseThemePendingChoice;
use crate::modules::user::stores::user::user_actions::user_actions::UserActionsStore;
use crate::modules::user::stores::user::user_services::user_services::UserServicesStore;

impl UserActionsStore {
    pub fn set_launch_animation_enabled_action(&mut self, enabled: bool, cx: &mut Context<Self>) {
        RiseLaunchPreferences::shared(cx).update(cx, |preferences, cx| preferences.set_animation_enabled(enabled, cx));
    }

    pub fn load_theme_settings_action(&mut self, force: bool, cx: &mut Context<Self>) {
        let account_id = SaiAccountCacheScope::active_user_id();
        let account_epoch = SaiAccountCacheScope::active_epoch();
        let domain = RiseUserActionExecutor::require_domain(cx);
        cx.spawn(async move |_, cx: &mut AsyncApp| {
            let result = match domain.await {
                Ok(domain) => domain.load_theme_settings(force).await,
                Err(error) => Err(error),
            };
            if let Err(error) = result {
                cx.update(|cx| {
                    Self::theme_settings_action_failed(
                        &error,
                        account_id.as_deref(),
                        account_epoch.as_deref(),
                        "theme settings load failed",
                        cx,
                    );
                });
            }
        })
        .detach();
    }

    pub fn set_theme_appearance_action(
        &mut self,
        appearance: Option<RiseThemeAppearance>,
        platform: RiseThemePlatform,
        cx: &mut Context<Self>,
    ) {
        let account_id = SaiAccountCacheScope::active_user_id();
        let account_epoch = SaiAccountCacheScope::active_epoch();
        if let Some(account_id) = account_id.as_deref() {
            RiseThemePendingChoice::remember(appearance, platform, account_id);
            let (has_value, service_preferences) = UserServicesStore::shared(cx)
                .update(cx, |services, cx| (services.theme_settings_has_value(cx), services.theme_preferences(cx)));
            let theme = ThemeStore::shared(cx);
            let current = if has_value { service_preferences } else { theme.read(cx).preferences() };
            let chosen = current.setting(appearance, platform);
            let installed = theme.update(cx, |theme, cx| theme.install(chosen, account_id, cx));
            let (resolved, ink) = {
                let store = theme.read(cx);
                (store.resolved_appearance(), store.ink_appearance())
            };
            Sai::log(
                "User",
                move || format!("theme choice {platform:?} {appearance:?}: installed {installed}, resolved {resolved:?}, ink {ink:?}"),
                SaiLogKind::Debug,
                cx,
            );
            RiseUserPresentation::shared(cx).update(cx, |presentation, cx| {
                presentation.apply_local_choice(chosen, account_id, cx);
            });
        }
        let domain = RiseUserActionExecutor::require_domain(cx);
        cx.spawn(async move |_, cx: &mut AsyncApp| {
            let result = match domain.await {
                Ok(domain) => domain.set_theme_appearance(appearance, platform).await,
                Err(error) => Err(error),
            };
            if let Err(error) = result {
                cx.update(|cx| {
                    Self::theme_settings_action_failed(
                        &error,
                        account_id.as_deref(),
                        account_epoch.as_deref(),
                        "theme settings update failed",
                        cx,
                    );
                });
            }
        })
        .detach();
    }

    fn theme_settings_action_failed(
        error: &anyhow::Error,
        account_id: Option<&str>,
        account_epoch: Option<&str>,
        label: &'static str,
        cx: &mut App,
    ) {
        RiseUserPresentation::shared(cx).update(cx, |presentation, cx| {
            presentation.report_theme_settings_failure(error, account_id, account_epoch, cx);
        });
        let reason = RiseUserDebugTrace::error_label(error);
        Sai::log("User", move || format!("{label}: {reason}"), SaiLogKind::Network, cx);
    }
}
