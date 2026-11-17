use gpui::{App, Hsla, IntoElement, ParentElement, RenderOnce, Window, div, px};

use crate::IconUi;

const BOT_ICON_SYMBOL: &str = "cpu";

/// the chip glyph beside a bot name
#[derive(IntoElement)]
pub struct BotIcon {
    size: f32,
    color: Hsla,
}

impl BotIcon {
    pub fn new(color: Hsla) -> Self {
        Self { size: 13.0, color }
    }

    pub fn size(mut self, size: f32) -> Self {
        self.size = size;
        self
    }
}

impl RenderOnce for BotIcon {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        div().children(IconUi::sized(BOT_ICON_SYMBOL, px(self.size), self.color, false))
    }
}
