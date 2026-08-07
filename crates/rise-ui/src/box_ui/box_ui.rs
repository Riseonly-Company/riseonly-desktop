use gpui::{Div, div, prelude::*};
use rise_theme::AppTheme;

/// The two containers every screen is built out of.
///
/// Both return a bare `Div` with nothing but theme values applied, so a call
/// site keeps styling on top of them with the usual gpui builders —
/// `BoxUi::surface(theme).p_4().child(..)`. Nothing here owns layout: a helper
/// that also picked padding and direction would be copied-and-tweaked at the
/// first screen that disagreed, and the shared part — which background sits on
/// which — would stop being shared.
pub struct BoxUi;

impl BoxUi {
    /// A card: one step lighter than the screen it sits on, with a hairline
    /// border.
    ///
    /// The lift is carried by `bg._200` against `bg._000`, not by a shadow, so
    /// it costs nothing to move — section 13 of the performance guide rules out
    /// unbounded shadows on anything that animates.
    pub fn surface(theme: &AppTheme) -> Div {
        div()
            .bg(theme.bg._200)
            .border_1()
            .border_color(theme.border._100)
            .rounded(theme.radius._300)
    }

    /// The root of a window or a page: fills its parent and establishes the
    /// inherited text colour.
    ///
    /// Setting `text_color` once here is what lets every label below it stay
    /// silent about colour and still be readable in both themes.
    pub fn screen(theme: &AppTheme) -> Div {
        div()
            .size_full()
            .bg(theme.bg._000)
            .text_color(theme.text.primary)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_surface_sits_above_the_screen_background() {
        let theme = AppTheme::dark();
        assert_ne!(theme.bg._000, theme.bg._200);
    }
}
