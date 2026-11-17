use gpui::{App, IntoElement, ParentElement, RenderOnce, Styled, Window, div, px};
use rise_ui::lottie_view::LottieView;

/// the cool_emoji lottie in a square frame
#[derive(IntoElement, Clone, Debug, PartialEq)]
pub struct CoolEmojiAnimation {
    pub size: f32,
}

impl Default for CoolEmojiAnimation {
    fn default() -> Self {
        Self { size: 100.0 }
    }
}

impl CoolEmojiAnimation {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn size(mut self, size: f32) -> Self {
        self.size = size;
        self
    }
}

impl RenderOnce for CoolEmojiAnimation {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let side = px(self.size);
        div()
            .w(side)
            .h(side)
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .child(LottieView::new("cool_emoji").size(self.size))
    }
}
