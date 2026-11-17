use gpui::{App, IntoElement, RenderOnce, Window};
use rise_theme::theme::theme_block_color::ThemeBlockColor;

use crate::modules::user::pages::profile::settings::customization_settings::settings::color_picker_screen::{
    ColorPagePresets, ColorPickerScreen, ColorPreviewKind,
};

#[derive(IntoElement, Default)]
pub struct BlockColorSettings;

impl BlockColorSettings {
    pub fn new() -> Self {
        Self
    }
}

impl RenderOnce for BlockColorSettings {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        ColorPickerScreen::new(
            "settings_block_color",
            ColorPreviewKind::Background,
            ColorPagePresets::surfaces(),
            |palette| palette.block_hex(),
            |palette, hex| palette.bg_200 = hex,
        )
    }
}
