use gpui::{App, IntoElement, ParentElement, RenderOnce, Styled, Window, div, px};
use rise_ui::lottie_view::LottieView;

/// the party lottie in a square frame
#[derive(IntoElement, Clone, Debug, PartialEq)]
pub struct PartyAnimation {
    pub size: f32,
}

impl Default for PartyAnimation {
    fn default() -> Self {
        Self { size: 100.0 }
    }
}

impl PartyAnimation {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn size(mut self, size: f32) -> Self {
        self.size = size;
        self
    }
}

impl RenderOnce for PartyAnimation {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let side = px(self.size);
        div()
            .w(side)
            .h(side)
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .child(LottieView::new("party").size(self.size))
    }
}
