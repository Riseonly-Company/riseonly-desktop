use gpui::{App, IntoElement, RenderOnce, Window};

use crate::modules::user::pages::profile::settings::customization_settings::settings::color_picker_screen::{
    ColorPagePresets, ColorPickerScreen, ColorPreviewKind,
};

#[derive(IntoElement, Default)]
pub struct BtnColorSettings;

impl BtnColorSettings {
    pub fn new() -> Self {
        Self
    }
}

impl RenderOnce for BtnColorSettings {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        ColorPickerScreen::new(
            "settings_btn_color",
            ColorPreviewKind::Button,
            ColorPagePresets::surfaces(),
            |palette| palette.btn_bg_300.clone(),
            |palette, hex| palette.btn_bg_300 = hex,
        )
    }
}
