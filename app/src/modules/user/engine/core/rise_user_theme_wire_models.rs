use rise_canon::rise_view::user::ThemeSettings;
use rise_theme::{RiseThemeAppearance, RiseThemePreferences};

/// prefs accessor for the canon theme settings type, a trait since the type is foreign to this crate
pub trait ThemeSettingsPreferences {
    /// unknown value reads as not chosen, same as the servers default
    fn preferences(&self) -> RiseThemePreferences;
}

impl ThemeSettingsPreferences for ThemeSettings {
    fn preferences(&self) -> RiseThemePreferences {
        RiseThemePreferences::new(
            self.mobile_theme.as_deref().and_then(RiseThemeAppearance::from_raw_value),
            self.desktop_theme.as_deref().and_then(RiseThemeAppearance::from_raw_value),
            self.web_theme.as_deref().and_then(RiseThemeAppearance::from_raw_value),
        )
    }
}
