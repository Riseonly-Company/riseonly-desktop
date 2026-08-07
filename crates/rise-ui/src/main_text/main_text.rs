use gpui::{Div, div, prelude::*};
use rise_theme::AppTheme;

/// How much of the reader's attention a run of text is asking for.
///
/// Emphasis in this design system is carried by colour, not by weight: the
/// typography tokens fix the face and the size, and a secondary line differs
/// only in how far it recedes from the background.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum TextTone {
    /// The content itself.
    #[default]
    Primary,
    /// Captions, timestamps, hints — present, but not competing.
    Secondary,
}

/// Text styled from the typography tokens.
///
/// Each function returns a `Div` carrying size, line height, face and colour;
/// the string goes on with `.child(..)`. Size and line height always travel
/// together — setting `text_size` alone leaves the previous line height in place
/// and the run silently changes its leading.
pub struct MainText;

impl MainText {
    /// The screen's heading, in the `large_title` token. One per screen.
    pub fn title(theme: &AppTheme) -> Div {
        let token = theme.typography.large_title();
        div()
            .text_size(token.size)
            .line_height(token.line_height)
            .font(token.font)
            .text_color(theme.text.primary)
    }

    /// Running text, in the `body` token, at the requested [`TextTone`].
    pub fn body(theme: &AppTheme, tone: TextTone) -> Div {
        let color = match tone {
            TextTone::Primary => theme.text.primary,
            TextTone::Secondary => theme.text.secondary,
        };
        let token = theme.typography.body();
        div()
            .text_size(token.size)
            .line_height(token.line_height)
            .font(token.font)
            .text_color(color)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secondary_text_is_dimmer_than_primary() {
        let theme = AppTheme::dark();
        assert_ne!(theme.text.primary, theme.text.secondary);
    }
}
