use gpui::{Div, div, prelude::*};
use rise_theme::AppTheme;

/// The two containers every screen is built out of. Both return a bare `Div`
/// carrying theme values only — no layout — so the call site styles on top:
/// `BoxUi::surface(theme).p_4().child(..)`.
pub struct BoxUi;

impl BoxUi {
    /// A card: one step lighter than the screen it sits on, with a hairline
    /// border. The lift is a background step, never a shadow.
    pub fn surface(theme: &AppTheme) -> Div {
        div()
            .bg(theme.bg._200)
            .border_1()
            .border_color(theme.border._100)
            .rounded(theme.radius._300)
    }

    /// The root of a page: fills its parent and sets the inherited text colour,
    /// so labels below it need not name one.
    ///
    /// It paints NO background. The shell's plate owns the app's colour, and a
    /// page that repaints it fills its own square bounds — which, at the plate's
    /// edge, squares off the rounded corner from the inside, because gpui clips
    /// to a rectangle and never to a radius. A page that genuinely needs a shade
    /// of its own says so at the call site.
    pub fn screen(theme: &AppTheme) -> Div {
        div().size_full().text_color(theme.text.primary)
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
