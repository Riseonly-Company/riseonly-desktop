use gpui::{App, Context, Entity};
use rise_platform::user_defaults::UserDefaults;
use rise_platform::user_defaults_keys::LAUNCH_ANIMATION_ENABLED;

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
            .unwrap_or(true);
        Self { animation_enabled }
    }

    pub fn animation_enabled(&self) -> bool {
        self.animation_enabled
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
