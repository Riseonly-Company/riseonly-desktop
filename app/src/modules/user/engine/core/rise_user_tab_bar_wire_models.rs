use rise_canon::RiseTabBarButton;
use rise_canon::rise_request::user::UpdateTabBarSettings;
use rise_canon::rise_view::user::{TabBarPlatform, TabBarSet};

use crate::core::widgets::navigations::main_tab_bar_settings::{
    RiseTabBarPlatform, RiseTabBarSettings, RiseTabBarSettingsChange, RiseTabBarSettingsSet, RiseTabBarSettle,
    RiseTabBarSurface,
};

pub trait UpdateTabBarSettingsChange: Sized {
    fn from_change(platform: RiseTabBarPlatform, change: RiseTabBarSettingsChange) -> Self;
}

impl UpdateTabBarSettingsChange for UpdateTabBarSettings {
    fn from_change(platform: RiseTabBarPlatform, change: RiseTabBarSettingsChange) -> Self {
        let (settings, reset) = match change {
            RiseTabBarSettingsChange::Set(chosen) => (chosen, false),
            RiseTabBarSettingsChange::Reset => (RiseTabBarSettings::defaults(), true),
        };
        Self {
            loops: settings.loops,
            more_tab: settings.more_tab,
            platform: platform.raw_value().to_owned(),
            return_delay_ms: settings.return_delay_ms,
            scroll_threshold_percent: settings.scroll_threshold_percent,
            speed_tenths: settings.speed_tenths,
            surface: settings.surface.raw_value().to_owned(),
            swipe_settle: settings.swipe_settle.raw_value().to_owned(),
            tap_settle: settings.tap_settle.raw_value().to_owned(),
            throw_back: settings.throw_back,
            throw_selects_first: settings.throw_selects_first,
            throw_speed: settings.throw_speed,
            button_order: Some(settings.button_order.iter().map(|button| button.wire().to_owned()).collect()),
            reset: Some(reset),
        }
    }
}

pub trait TabBarPlatformSettings {
    /// closed lists arrive as wire strings, an unknown value reads as the default instead of failing
    fn settings(&self) -> RiseTabBarSettings;
}

impl TabBarPlatformSettings for TabBarPlatform {
    fn settings(&self) -> RiseTabBarSettings {
        let base = RiseTabBarSettings::defaults();
        RiseTabBarSettings::new(
            self.more_tab,
            RiseTabBarSurface::from_raw_value(&self.surface).unwrap_or(base.surface),
            self.loops,
            self.scroll_threshold_percent,
            self.speed_tenths,
            RiseTabBarSettle::from_raw_value(&self.tap_settle).unwrap_or(base.tap_settle),
            RiseTabBarSettle::from_raw_value(&self.swipe_settle).unwrap_or(base.swipe_settle),
            self.return_delay_ms,
            self.throw_back,
            self.throw_speed,
            self.throw_selects_first,
            &self.button_order.iter().filter_map(|raw| RiseTabBarButton::from_wire(raw)).collect::<Vec<_>>(),
        )
        .clamped()
    }
}

pub trait TabBarSetSettings {
    fn settings(&self) -> Option<RiseTabBarSettingsSet>;
}

impl TabBarSetSettings for TabBarSet {
    fn settings(&self) -> Option<RiseTabBarSettingsSet> {
        let (Some(ios), Some(android)) = (&self.ios, &self.android) else {
            return None;
        };
        Some(RiseTabBarSettingsSet { ios: ios.settings(), android: android.settings() })
    }
}
