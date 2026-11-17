use gpui::{App, Window};
use rise_theme::ThemePalette;
use rise_ui::swiftui::prelude::*;

use crate::modules::user::pages::profile::settings::customization_settings::settings::color_picker_screen::{
    ColorPagePresets, ColorPickerScreen, ColorPreviewKind,
};

#[derive(IntoElement)]
pub struct PrimaryColorSettings;

impl PrimaryColorSettings {
    pub fn new() -> Self {
        Self
    }
}

impl RenderOnce for PrimaryColorSettings {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        ColorPickerScreen::new(
            "settings_primary_color",
            ColorPreviewKind::Primary,
            ColorPagePresets::accents(),
            |palette: &ThemePalette| palette.primary_100.clone(),
            |palette: &mut ThemePalette, hex: String| {
                palette.primary_100 = hex.clone();
                palette.primary_200 = hex.clone();
                palette.primary_300 = hex;
            },
        )
    }
}
