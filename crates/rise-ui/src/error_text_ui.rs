use gpui::{App, Hsla, IntoElement, ParentElement, RenderOnce, SharedString, Styled, Window, div};
use rise_theme::ui_color_system::UIColor;
use rise_theme::{Appearance, RiseThemeAppearance, ThemeStore};

use crate::swiftui::font::{Font, SwiftUiFont};
use crate::swiftui::frame::{EdgeSet, SwiftUiFrame};

/// a field validation message under it
#[derive(IntoElement)]
pub struct ErrorTextUi {
    text: SharedString,
    px: f32,
}

impl ErrorTextUi {
    pub fn new(text: impl Into<SharedString>) -> Self {
        Self {
            text: text.into(),
            px: 14.0,
        }
    }

    pub fn px(mut self, px: f32) -> Self {
        self.px = px;
        self
    }
}

impl RenderOnce for ErrorTextUi {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let text_delta = ThemeStore::shared(cx).read(cx).text_delta();
        let red = system_red(cx);
        div()
            .swiftui_font(Font::system_size((self.px + text_delta).max(1.0)))
            .text_color(red)
            .padding_edges(EdgeSet::LEADING, 7.0)
            .padding_edges(EdgeSet::TOP, 2.0)
            .child(self.text)
    }
}

pub fn system_red(cx: &mut App) -> Hsla {
    let appearance = match ThemeStore::shared(cx).read(cx).resolved_appearance() {
        RiseThemeAppearance::Light => Appearance::Light,
        RiseThemeAppearance::Dark => Appearance::Dark,
    };
    UIColor::system_red().resolved_color(appearance)
}
