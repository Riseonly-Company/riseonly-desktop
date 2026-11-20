use gpui::{App, Context, Entity};
use rise_platform::app_info::AppInfo;
use rise_platform::user_defaults::UserDefaults;
use rise_platform::user_defaults_keys::{LAUNCH_ANIMATION_ENABLED, LAUNCH_ANIMATION_PLAYED_BUILD};

use crate::core::config::app_environment::AppEnvironment;

pub struct RiseLaunchPreferences {
    animation_enabled: bool,
}

impl RiseLaunchPreferences {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, |_| Self::new())
    }

    fn new() -> Self {
        let animation_enabled = UserDefaults::standard()
            .object(LAUNCH_ANIMATION_ENABLED)
            .and_then(|value| value.as_bool())
            .unwrap_or(false);
        Self { animation_enabled }
    }

    pub fn animation_enabled(&self) -> bool {
        self.animation_enabled
    }

    /// the art plays always when the toggle is on, else once after an install and once after each update
    pub fn plays_at_launch(&self) -> bool {
        let defaults = UserDefaults::standard();
        let build = Self::installed_build();
        let first_of_build = defaults.string(LAUNCH_ANIMATION_PLAYED_BUILD).as_deref() != Some(build.as_str());
        if first_of_build {
            defaults.set(LAUNCH_ANIMATION_PLAYED_BUILD, build);
        }
        self.animation_enabled || first_of_build
    }

    fn installed_build() -> String {
        AppInfo::current(AppEnvironment::current().as_str())
            .map(|info| format!("{} ({})", info.short_version.trim(), info.build.trim()))
            .unwrap_or_default()
    }

    pub fn set_animation_enabled(&mut self, enabled: bool, cx: &mut Context<Self>) {
        if self.animation_enabled == enabled {
            return;
        }
        self.animation_enabled = enabled;
        UserDefaults::standard().set(LAUNCH_ANIMATION_ENABLED, enabled);
        cx.notify();
    }
}
