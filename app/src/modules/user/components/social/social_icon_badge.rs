use gpui::{App, Div, Hsla, IntoElement, ParentElement, RenderOnce, SharedString, Styled, Window, div, px, svg, white};
use rise_ui::icon_ui::IconUi;
use rise_ui::swiftui::shapes::SwiftUiShapes;
use rise_ui::swiftui::stacks::{Alignment, zstack};

use crate::modules::user::shared::social_platform::SocialPlatform;

const SOCIAL_ASSET_PREFIX: &str = "images/social/";

#[derive(IntoElement)]
pub struct SocialIconBadge {
    pub platform: SocialPlatform,
    pub size: f32,
}

impl SocialIconBadge {
    pub fn new(platform: SocialPlatform) -> Self {
        Self { platform, size: 34.0 }
    }

    pub fn size(mut self, size: f32) -> Self {
        self.size = size;
        self
    }

    fn tile(&self) -> Div {
        if self.platform.uses_gradient() {
            div().size_full().child(self.platform.tile_gradient().view())
        } else {
            div().size_full().bg(self.platform.tile_color())
        }
    }

    fn glyph(&self) -> Div {
        let size = self.size;
        let color: Hsla = self.platform.glyph_color();
        let glyph = div().flex().items_center().justify_center();
        if let Some(asset) = self.platform.asset_name() {
            let side = px(size * 0.54);
            glyph.child(
                svg().path(SharedString::from(format!("{SOCIAL_ASSET_PREFIX}{asset}.svg"))).size(side).text_color(color),
            )
        } else {
            glyph.children(IconUi::sized(self.platform.system_fallback_icon(), px(size * 0.5), color, false))
        }
    }
}

impl RenderOnce for SocialIconBadge {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let radius = px(self.size * 0.3);
        zstack(Alignment::Center)
            .size(px(self.size))
            .flex_none()
            .rounded(radius)
            .overflow_hidden()
            .stroke_border(white().opacity(0.12), 0.5)
            .child(self.tile())
            .child(self.glyph())
    }
}
