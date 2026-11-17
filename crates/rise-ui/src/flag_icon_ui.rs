use gpui::{App, IntoElement, ParentElement, RenderOnce, SharedString, Window, div};

use crate::swiftui::font::{Font, SwiftUiFont};
use crate::swiftui::frame::SwiftUiAlignedFrame;
use crate::swiftui::stacks::Alignment;

/// emoji flag sized to sit in a GroupedBtn leading slot
/// matches the symbol tiles of other settings rows
#[derive(IntoElement)]
pub struct FlagIconUi {
    flag: SharedString,
}

impl FlagIconUi {
    pub fn new(flag: impl Into<SharedString>) -> Self {
        Self { flag: flag.into() }
    }
}

impl RenderOnce for FlagIconUi {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        div()
            .swiftui_font(Font::system_size(27.0))
            .child(self.flag)
            .frame_size_aligned(30.0, 30.0, Alignment::Center)
    }
}
