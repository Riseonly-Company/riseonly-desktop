use gpui::{App, FontWeight, SharedString, Window};
use rise_theme::color;
use rise_ui::main_text::MainText;
use rise_ui::swiftui::prelude::*;

#[derive(IntoElement)]
pub struct MemberTagChip {
    title: SharedString,
    color_hex: SharedString,
}

impl MemberTagChip {
    pub fn new(title: impl Into<SharedString>) -> Self {
        Self { title: title.into(), color_hex: SharedString::default() }
    }

    pub fn color_hex(mut self, color_hex: impl Into<SharedString>) -> Self {
        self.color_hex = color_hex.into();
        self
    }
}

impl RenderOnce for MemberTagChip {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx);
        let tint = if self.color_hex.is_empty() { theme.primary._100 } else { theme.legible(color(&self.color_hex)) };
        div()
            .px(px(7.0))
            .py(px(2.0))
            .overflow_hidden()
            .bg(tint.opacity(0.16))
            .rounded_full()
            .child(MainText::new(self.title).px(11.0).font_weight(FontWeight::SEMIBOLD).color(tint).number_of_lines(1))
    }
}
