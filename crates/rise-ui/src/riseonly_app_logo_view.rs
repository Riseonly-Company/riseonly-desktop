use gpui::{App, IntoElement, ObjectFit, RenderOnce, Styled, StyledImage, Window, img, px};

use crate::swiftui::shapes::SwiftUiShapes;

const APP_LOGO_ASSET: &str = "images/AppLogo.png";

#[derive(IntoElement)]
pub struct RiseonlyAppLogoView {
    size: f32,
}

impl Default for RiseonlyAppLogoView {
    fn default() -> Self {
        Self { size: 28.0 }
    }
}

impl RiseonlyAppLogoView {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn size(mut self, size: f32) -> Self {
        self.size = size;
        self
    }

    fn corner_radius(&self) -> f32 {
        self.size * (8.0 / 28.0)
    }
}

impl RenderOnce for RiseonlyAppLogoView {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let corner_radius = self.corner_radius();
        img(APP_LOGO_ASSET)
            .object_fit(ObjectFit::Contain)
            .w(px(self.size))
            .h(px(self.size))
            .continuous_corners(corner_radius)
            .overflow_hidden()
    }
}
