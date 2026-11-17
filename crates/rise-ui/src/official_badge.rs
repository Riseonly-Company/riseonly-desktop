use gpui::{App, Hsla, IntoElement, Pixels, RenderOnce, Rgba, Styled, Window, div, px};

use crate::icon_ui::IconUi;

const OFFICIAL_BADGE_SYMBOL: &str = "checkmark.seal.fill";

/// the verified seal beside a name
#[derive(IntoElement, Clone, Copy, Debug, PartialEq)]
pub struct OfficialBadge {
    pub size: Pixels,
    pub color: Hsla,
}

impl Default for OfficialBadge {
    fn default() -> Self {
        Self {
            size: px(16.0),
            color: Rgba {
                r: 1.0,
                g: 0.353,
                b: 0.353,
                a: 1.0,
            }
            .into(),
        }
    }
}

impl OfficialBadge {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn size(mut self, size: Pixels) -> Self {
        self.size = size;
        self
    }

    pub fn color(mut self, color: impl Into<Hsla>) -> Self {
        self.color = color.into();
        self
    }
}

impl RenderOnce for OfficialBadge {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        match IconUi::sized(OFFICIAL_BADGE_SYMBOL, self.size, self.color, true) {
            Some(seal) => seal.into_any_element(),
            None => div().size(self.size).into_any_element(),
        }
    }
}
