use gpui::{App, IntoElement, RenderOnce, Window};
use rise_theme::ThemePalette;

use crate::modules::user::pages::profile::settings::customization_settings::settings::color_picker_screen::{
    ColorPagePresets, ColorPickerScreen, ColorPreviewKind,
};

#[derive(IntoElement)]
pub struct TextColorSettings;

impl TextColorSettings {
    pub fn new() -> Self {
        Self
    }
}

impl RenderOnce for TextColorSettings {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        ColorPickerScreen::new(
            "settings_text_color",
            ColorPreviewKind::Text,
            ColorPagePresets::inks(),
            |palette: &ThemePalette| palette.text_100.clone(),
            |palette: &mut ThemePalette, hex: String| palette.text_100 = hex,
        )
    }
}
