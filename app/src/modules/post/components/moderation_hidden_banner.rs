use gpui::{App, FontWeight, IntoElement, ParentElement, RenderOnce, Styled, Window, px};
use rise_theme::ThemeStore;
use rise_ui::icon_ui::IconUi;
use rise_ui::main_text::MainText;
use rise_ui::swiftui::frame::{EdgeSet, SwiftUiFrame};
use rise_ui::swiftui::shapes::SwiftUiShapes;
use rise_ui::swiftui::stacks::{hstack, spacer_min_w};

/// hidden by moderation plate an author sees on their own disabled object
/// one plate for every kind so it reads the same on a vacancy card and a post
#[derive(IntoElement)]
pub struct ModerationHiddenBanner {
    pub text: String,
}

impl ModerationHiddenBanner {
    pub fn new(text: impl Into<String>) -> Self {
        Self { text: text.into() }
    }
}

impl RenderOnce for ModerationHiddenBanner {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let error = ThemeStore::shared(cx).read(cx).current_theme().semantic.error_100;
        hstack(6.0)
            .children(IconUi::sized("eye.slash", px(12.0), error, false))
            .child(MainText::new(self.text).px(12.0).font_weight(FontWeight::MEDIUM).color(error).number_of_lines(1))
            .child(spacer_min_w(0.0))
            .padding_edges(EdgeSet::HORIZONTAL, 10.0)
            .padding_edges(EdgeSet::VERTICAL, 7.0)
            .bg(error.opacity(0.1))
            .continuous_corners(8.0)
            .overflow_hidden()
    }
}
