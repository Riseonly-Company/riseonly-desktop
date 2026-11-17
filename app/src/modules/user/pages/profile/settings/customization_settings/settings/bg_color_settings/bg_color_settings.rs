use gpui::{App, IntoElement, RenderOnce, Window};

use crate::modules::user::pages::profile::settings::customization_settings::settings::color_picker_screen::{
    ColorPagePresets, ColorPickerScreen, ColorPreviewKind,
};

#[derive(IntoElement, Default)]
pub struct BgColorSettings;

impl BgColorSettings {
    pub fn new() -> Self {
        Self
    }
}

impl RenderOnce for BgColorSettings {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        ColorPickerScreen::new(
            "settings_bg_color",
            ColorPreviewKind::Background,
            ColorPagePresets::surfaces(),
            |palette| palette.bg_100.clone(),
            |palette, hex| palette.bg_100 = hex,
        )
    }
}
