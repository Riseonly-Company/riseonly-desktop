use gpui::{App, Window};
use rise_theme::ThemePalette;
use rise_ui::swiftui::prelude::*;

use crate::modules::user::pages::profile::settings::customization_settings::settings::color_picker_screen::{
    ColorPagePresets, ColorPickerScreen, ColorPreviewKind,
};

#[derive(IntoElement)]
pub struct SecondaryTextColorSettings;

impl SecondaryTextColorSettings {
    pub fn new() -> Self {
        Self
    }
}

impl RenderOnce for SecondaryTextColorSettings {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        ColorPickerScreen::new(
            "settings_secondary_text_color",
            ColorPreviewKind::SecondaryText,
            ColorPagePresets::inks(),
            |palette: &ThemePalette| palette.secondary_100.clone(),
            |palette: &mut ThemePalette, hex: String| palette.secondary_100 = hex,
        )
    }
}
