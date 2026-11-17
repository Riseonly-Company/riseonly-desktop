use gpui::{App, IntoElement, ParentElement, RenderOnce, Styled, Window, div, px};
use rise_ui::lottie_view::LottieView;

/// the magic_crystal_ball lottie in a square frame
#[derive(IntoElement, Clone, Debug, PartialEq)]
pub struct MagicCrystalBallAnimation {
    pub size: f32,
}

impl Default for MagicCrystalBallAnimation {
    fn default() -> Self {
        Self { size: 100.0 }
    }
}

impl MagicCrystalBallAnimation {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn size(mut self, size: f32) -> Self {
        self.size = size;
        self
    }
}

impl RenderOnce for MagicCrystalBallAnimation {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let side = px(self.size);
        div()
            .w(side)
            .h(side)
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .child(LottieView::new("magic_crystal_ball").size(self.size))
    }
}
