use rise_theme::{RiseThemeAppearance, ThemePalette};

use crate::modules::theme::stores::theme_presets::theme_presets_services::theme_presets_services::ThemePreset;

#[test]
fn only_light_changed_reads_as_light() {
    let mut light = ThemePalette::default_light();
    light.bg_100 = "#F2E8FF".to_owned();
    assert_eq!(ThemePreset::mode_for(&light, &ThemePalette::default_dark()), RiseThemeAppearance::Light);
}

#[test]
fn only_dark_changed_reads_as_dark() {
    let mut dark = ThemePalette::default_dark();
    dark.bg_100 = "#1B0F2E".to_owned();
    assert_eq!(ThemePreset::mode_for(&ThemePalette::default_light(), &dark), RiseThemeAppearance::Dark);
}

#[test]
fn both_changed_take_the_larger_and_a_tie_reads_as_dark() {
    let mut light = ThemePalette::default_light();
    light.bg_100 = "#F2E8FF".to_owned();
    light.primary_100 = "#7E57F7".to_owned();
    let mut dark = ThemePalette::default_dark();
    dark.bg_100 = "#1B0F2E".to_owned();
    assert_eq!(ThemePreset::mode_for(&light, &dark), RiseThemeAppearance::Light);

    dark.primary_100 = "#9B7BFF".to_owned();
    assert_eq!(ThemePreset::mode_for(&light, &dark), RiseThemeAppearance::Dark);
}
