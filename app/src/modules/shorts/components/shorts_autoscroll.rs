use std::time::Duration;

use gpui::{App, Context, Entity};
use rise_canon::RiseLimits;
use rise_platform::user_defaults::UserDefaults;
use rise_platform::user_defaults_keys::SHORTS_AUTOSCROLL_ENABLED;

use crate::core::lib::canon_rules::RiseLimitsCanonRules;

/// device wide switch for riffs that move on by themselves
pub struct ShortsAutoscrollPreferences {
    is_enabled: bool,
}

impl ShortsAutoscrollPreferences {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self { is_enabled: UserDefaults::standard().bool(SHORTS_AUTOSCROLL_ENABLED) }
    }

    pub fn is_enabled(&self) -> bool {
        self.is_enabled
    }

    pub fn global_is_enabled(cx: &App) -> bool {
        rise_ui::shared::try_read::<Self>(cx).is_some_and(|preferences| preferences.is_enabled)
    }

    pub fn set_enabled(&mut self, enabled: bool, cx: &mut Context<Self>) {
        if self.is_enabled == enabled {
            return;
        }
        self.is_enabled = enabled;
        UserDefaults::standard().set(SHORTS_AUTOSCROLL_ENABLED, enabled);
        cx.notify();
    }
}

pub enum ShortsAutoscroll {}

impl ShortsAutoscroll {
    pub fn photo_milliseconds() -> i64 {
        RiseLimits::client_value("shorts_autoscroll_photo_ms")
    }

    pub fn photo_duration() -> Duration {
        Duration::from_millis(u64::try_from(Self::photo_milliseconds()).unwrap_or(0))
    }

    pub fn photo_seconds() -> i64 {
        (Self::photo_milliseconds() / 1000).max(1)
    }
}
